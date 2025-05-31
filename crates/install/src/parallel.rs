//! Parallel package execution with dependency ordering

// InstallContext import removed as it's not used in this module
use crossbeam::queue::SegQueue;
use dashmap::DashMap;
use spsv2_errors::{Error, InstallError};
use spsv2_events::{Event, EventSender};
use spsv2_net::NetClient;
use spsv2_resolver::{ExecutionPlan, NodeAction, PackageId, ResolvedNode};
use spsv2_store::PackageStore;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

/// Parallel executor for package operations
pub struct ParallelExecutor {
    /// Network client for downloads
    net_client: NetClient,
    /// Package store
    store: PackageStore,
    /// Maximum concurrent operations
    max_concurrency: usize,
    /// Download timeout
    download_timeout: Duration,
}

impl ParallelExecutor {
    /// Create new parallel executor
    ///
    /// # Errors
    ///
    /// Returns an error if network client initialization fails.
    pub fn new(store: PackageStore) -> Result<Self, Error> {
        Ok(Self {
            net_client: NetClient::with_defaults()?,
            store,
            max_concurrency: 4,                         // Default from spec
            download_timeout: Duration::from_secs(300), // 5 minutes
        })
    }

    /// Set maximum concurrency
    #[must_use]
    pub fn with_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    /// Set download timeout
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.download_timeout = timeout;
        self
    }

    /// Get maximum concurrency (for testing)
    #[cfg(test)]
    #[must_use]
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Get download timeout (for testing)
    #[cfg(test)]
    #[must_use]
    pub fn download_timeout(&self) -> Duration {
        self.download_timeout
    }

    /// Execute packages in parallel according to execution plan
    ///
    /// # Errors
    ///
    /// Returns an error if package processing fails, download fails, or concurrency limits are exceeded.
    pub async fn execute_parallel(
        &self,
        execution_plan: &ExecutionPlan,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
        context: &ExecutionContext,
    ) -> Result<(), Error> {
        let semaphore = Arc::new(Semaphore::new(self.max_concurrency));
        let ready_queue = Arc::new(SegQueue::new());
        let inflight = Arc::new(DashMap::new());
        let graph = Self::build_execution_graph(self, execution_plan, resolved_packages);

        // Initialize ready queue with packages that have no dependencies
        for package_id in execution_plan.ready_packages() {
            // Only add packages with in_degree 0 from our graph
            if let Some(node) = graph.get(&package_id) {
                if node.in_degree.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                    ready_queue.push(package_id);
                }
            } else {
                ready_queue.push(package_id);
            }
        }

        // Process packages until completion
        while !execution_plan.is_complete() || !inflight.is_empty() {
            // Try to start new tasks from ready queue
            while let Some(package_id) = ready_queue.pop() {
                if inflight.contains_key(&package_id) {
                    continue; // Already in flight
                }

                let permit = semaphore.clone().acquire_owned().await.map_err(|_| {
                    InstallError::ConcurrencyError {
                        message: "failed to acquire semaphore".to_string(),
                    }
                })?;

                let node = resolved_packages.get(&package_id).ok_or_else(|| {
                    InstallError::PackageNotFound {
                        package: package_id.name.clone(),
                    }
                })?;

                let handle = self.spawn_package_task(
                    package_id.clone(),
                    node.clone(),
                    context.clone(),
                    permit,
                );

                inflight.insert(package_id, handle);
            }

            // Wait for at least one task to complete
            if !inflight.is_empty() {
                let completed_package = self.wait_for_completion(&inflight).await?;

                // Notify execution plan and get newly ready packages
                let newly_ready = execution_plan.complete_package(&completed_package);
                for package_id in newly_ready {
                    ready_queue.push(package_id);
                }
            }

            // Small delay to prevent busy waiting
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }

    /// Build execution graph for tracking
    fn build_execution_graph(
        _self: &Self,
        execution_plan: &ExecutionPlan,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
    ) -> HashMap<PackageId, ExecutionNode> {
        let mut graph = HashMap::new();

        for package_id in resolved_packages.keys() {
            if let Some(metadata) = execution_plan.metadata(package_id) {
                let node = ExecutionNode {
                    action: metadata.action.clone(),
                    in_degree: AtomicUsize::new(metadata.in_degree()),
                    parents: metadata.parents.clone(),
                };
                graph.insert(package_id.clone(), node);
            }
        }

        graph
    }

    /// Spawn task for processing a single package
    fn spawn_package_task(
        &self,
        package_id: PackageId,
        node: ResolvedNode,
        context: ExecutionContext,
        _permit: tokio::sync::OwnedSemaphorePermit,
    ) -> JoinHandle<Result<PackageId, Error>> {
        let net_client = self.net_client.clone();
        let store = self.store.clone();
        let timeout_duration = self.download_timeout;

        tokio::spawn(async move {
            Self::process_package(
                package_id,
                node,
                context,
                net_client,
                store,
                timeout_duration,
            )
            .await
        })
    }

    /// Process a single package (download/local)
    async fn process_package(
        package_id: PackageId,
        node: ResolvedNode,
        context: ExecutionContext,
        net_client: NetClient,
        store: PackageStore,
        timeout_duration: Duration,
    ) -> Result<PackageId, Error> {
        context.send_event(Event::PackageInstalling {
            name: package_id.name.clone(),
            version: package_id.version.clone(),
        });

        match node.action {
            NodeAction::Download => {
                if let Some(url) = &node.url {
                    // Download package with timeout
                    let download_result = timeout(
                        timeout_duration,
                        Self::download_package(url, &package_id, &net_client, &store, &context),
                    )
                    .await;

                    match download_result {
                        Ok(Ok(())) => {
                            context.send_event(Event::PackageDownloaded {
                                name: package_id.name.clone(),
                                version: package_id.version.clone(),
                            });
                        }
                        Ok(Err(e)) => return Err(e),
                        Err(_) => {
                            return Err(InstallError::DownloadTimeout {
                                package: package_id.name.clone(),
                                url: url.clone(),
                                timeout_seconds: timeout_duration.as_secs(),
                            }
                            .into());
                        }
                    }
                } else {
                    return Err(InstallError::MissingDownloadUrl {
                        package: package_id.name.clone(),
                    }
                    .into());
                }
            }
            NodeAction::Local => {
                if let Some(path) = &node.path {
                    // Add local package to store
                    store.add_local_package(path).await?;

                    context.send_event(Event::PackageInstalled {
                        name: package_id.name.clone(),
                        version: package_id.version.clone(),
                        path: path.display().to_string(),
                    });
                } else {
                    return Err(InstallError::MissingLocalPath {
                        package: package_id.name.clone(),
                    }
                    .into());
                }
            }
        }

        Ok(package_id)
    }

    /// Download a package to the store
    async fn download_package(
        url: &str,
        package_id: &PackageId,
        net_client: &NetClient,
        store: &PackageStore,
        context: &ExecutionContext,
    ) -> Result<(), Error> {
        // Download to temporary file first
        let temp_file =
            tempfile::NamedTempFile::new().map_err(|e| InstallError::TempFileError {
                message: e.to_string(),
            })?;

        // Download with progress reporting
        net_client
            .download_file_with_progress(url, temp_file.path(), |progress| {
                context.send_event(Event::DownloadProgress {
                    url: url.to_string(),
                    bytes_downloaded: progress.downloaded,
                    total_bytes: progress.total,
                });
            })
            .await?;

        // Add to store
        store
            .add_package_from_file(temp_file.path(), &package_id.name, &package_id.version)
            .await?;

        Ok(())
    }

    /// Wait for at least one task to complete
    async fn wait_for_completion(
        &self,
        inflight: &DashMap<PackageId, JoinHandle<Result<PackageId, Error>>>,
    ) -> Result<PackageId, Error> {
        loop {
            // Check for completed tasks
            let mut completed = None;

            for entry in inflight {
                let package_id = entry.key();
                let handle = entry.value();

                if handle.is_finished() {
                    completed = Some(package_id.clone());
                    break;
                }
            }

            if let Some(package_id) = completed {
                if let Some((_, handle)) = inflight.remove(&package_id) {
                    match handle.await {
                        Ok(Ok(completed_package)) => return Ok(completed_package),
                        Ok(Err(e)) => return Err(e),
                        Err(e) => {
                            return Err(InstallError::TaskError {
                                message: format!("Task failed for {}: {}", package_id.name, e),
                            }
                            .into());
                        }
                    }
                }
            }

            // Small delay before checking again
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Execution context for parallel operations
#[derive(Clone)]
pub struct ExecutionContext {
    /// Event sender for progress reporting
    event_sender: Option<EventSender>,
}

impl ExecutionContext {
    /// Create new execution context
    #[must_use]
    pub fn new() -> Self {
        Self { event_sender: None }
    }

    /// Set event sender
    #[must_use]
    pub fn with_event_sender(mut self, event_sender: EventSender) -> Self {
        self.event_sender = Some(event_sender);
        self
    }

    /// Send event if sender is available
    pub fn send_event(&self, event: Event) {
        if let Some(sender) = &self.event_sender {
            let _ = sender.send(event);
        }
    }
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Execution node for tracking dependencies
struct ExecutionNode {
    /// Action to perform (for future use)
    #[allow(dead_code)]
    action: NodeAction,
    /// Remaining dependencies
    in_degree: AtomicUsize,
    /// Parent packages (for future dependency tracking)
    #[allow(dead_code)]
    parents: Vec<PackageId>,
}

/// Download progress information
#[cfg(test)]
pub struct DownloadProgress {
    /// Bytes downloaded
    pub downloaded: u64,
    /// Total bytes
    pub total: u64,
}

#[cfg(test)]
impl DownloadProgress {
    /// Calculate progress percentage
    pub fn percentage(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            {
                (self.downloaded as f64 / self.total as f64) * 100.0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Remove unused import
    use tempfile::tempdir;

    #[test]
    fn test_execution_context() {
        let context = ExecutionContext::new();
        assert!(context.event_sender.is_none());

        // Test event sending (should not panic)
        context.send_event(Event::PackageDownloaded {
            name: "test".to_string(),
            version: spsv2_types::Version::parse("1.0.0").unwrap(),
        });
    }

    #[test]
    fn test_download_progress() {
        let progress = DownloadProgress {
            downloaded: 50,
            total: 100,
        };

        assert!((progress.percentage() - 50.0).abs() < f64::EPSILON);

        let zero_total = DownloadProgress {
            downloaded: 10,
            total: 0,
        };

        assert!((zero_total.percentage() - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_parallel_executor_creation() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        let executor = ParallelExecutor::new(store)
            .unwrap()
            .with_concurrency(8)
            .with_timeout(Duration::from_secs(600));

        assert_eq!(executor.max_concurrency(), 8);
        assert_eq!(executor.download_timeout(), Duration::from_secs(600));
    }
}
