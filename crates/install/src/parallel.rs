//! Parallel package execution with dependency ordering

// InstallContext import removed as it's not used in this module
// validate_sp_file import removed - validation now handled by AtomicInstaller
use crate::PreparedPackage;
use crossbeam::queue::SegQueue;
use dashmap::DashMap;
use sps2_errors::{Error, InstallError};
use sps2_events::{Event, EventEmitter, EventSender};
use sps2_net::NetClient;
use sps2_resolver::{ExecutionPlan, NodeAction, PackageId, ResolvedNode};
use sps2_resources::ResourceManager;
use sps2_state::StateManager;
use sps2_store::PackageStore;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration, Instant};

struct ProcessPackageArgs {
    package_id: PackageId,
    node: ResolvedNode,
    context: ExecutionContext,
    net_client: NetClient,
    store: PackageStore,
    state_manager: StateManager,
    timeout_duration: Duration,
    prepared_packages: Arc<DashMap<PackageId, PreparedPackage>>,
}

/// Parallel executor for package operations
pub struct ParallelExecutor {
    /// Network client for downloads
    net_client: NetClient,
    /// Package store
    store: PackageStore,
    /// State manager for package_map updates
    state_manager: StateManager,
    /// Resource manager for concurrency control
    resources: Arc<ResourceManager>,
    /// Download timeout
    download_timeout: Duration,
}

impl ParallelExecutor {
    /// Create new parallel executor
    ///
    /// # Errors
    ///
    /// Returns an error if network client initialization fails.
    pub fn new(
        store: PackageStore,
        state_manager: StateManager,
        resources: Arc<ResourceManager>,
    ) -> Result<Self, Error> {
        Ok(Self {
            net_client: NetClient::with_defaults()?,
            store,
            state_manager,
            resources,
            download_timeout: Duration::from_secs(300), // 5 minutes
        })
    }

    /// Set download timeout
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.download_timeout = timeout;
        self
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
    ) -> Result<HashMap<PackageId, PreparedPackage>, Error> {
        let ready_queue = Arc::new(SegQueue::new());
        let inflight = Arc::new(DashMap::new());
        let prepared_packages = Arc::new(DashMap::new());
        let graph = Self::build_execution_graph(self, execution_plan, resolved_packages);

        // Initialize ready queue with packages that have no dependencies
        context.emit_event(Event::DebugLog {
            message: format!(
                "DEBUG: Execution plan has {} ready packages",
                execution_plan.ready_packages().len()
            ),
            context: std::collections::HashMap::from([(
                "ready_packages".to_string(),
                execution_plan
                    .ready_packages()
                    .iter()
                    .map(|id| format!("{}-{}", id.name, id.version))
                    .collect::<Vec<_>>()
                    .join(", "),
            )]),
        });

        for package_id in execution_plan.ready_packages() {
            context.emit_event(Event::DebugLog {
                message: format!(
                    "DEBUG: Processing ready package {}-{}",
                    package_id.name, package_id.version
                ),
                context: std::collections::HashMap::new(),
            });

            // Only add packages with in_degree 0 from our graph
            if let Some(node) = graph.get(&package_id) {
                let in_degree = node.in_degree.load(std::sync::atomic::Ordering::Relaxed);
                context.emit_event(Event::DebugLog {
                    message: format!(
                        "DEBUG: Package {}-{} has in_degree {}",
                        package_id.name, package_id.version, in_degree
                    ),
                    context: std::collections::HashMap::new(),
                });

                if in_degree == 0 {
                    ready_queue.push(package_id.clone());
                    context.emit_event(Event::DebugLog {
                        message: format!(
                            "DEBUG: Added package {}-{} to ready queue",
                            package_id.name, package_id.version
                        ),
                        context: std::collections::HashMap::new(),
                    });
                }
            } else {
                ready_queue.push(package_id.clone());
                context.emit_event(Event::DebugLog {
                    message: format!(
                        "DEBUG: Added package {}-{} to ready queue (not in graph)",
                        package_id.name, package_id.version
                    ),
                    context: std::collections::HashMap::new(),
                });
            }
        }

        // Process packages until completion with overall timeout
        let overall_timeout = Duration::from_secs(1800); // 30 minutes total
        let start_time = Instant::now();
        let mut no_progress_iterations = 0;
        let mut last_completed_count = 0;

        context.emit_event(Event::DebugLog {
            message: format!(
                "DEBUG: Starting main processing loop. execution_plan.is_complete()={}, inflight.is_empty()={}",
                execution_plan.is_complete(), inflight.is_empty()
            ),
            context: std::collections::HashMap::new(),
        });

        // Process packages until completion - ensure we process ready packages even if execution_plan reports complete
        while (!execution_plan.is_complete() || !inflight.is_empty()) || !ready_queue.is_empty() {
            context.emit_event(Event::DebugLog {
                message: format!(
                    "DEBUG: Loop iteration. execution_plan.is_complete()={}, inflight.is_empty()={}",
                    execution_plan.is_complete(), inflight.is_empty()
                ),
                context: std::collections::HashMap::new(),
            });
            // Check overall timeout
            if start_time.elapsed() > overall_timeout {
                return Err(InstallError::OperationTimeout {
                    message: "Overall installation timeout exceeded (30 minutes)".to_string(),
                }
                .into());
            }

            // Track progress to detect infinite loops
            let current_completed = execution_plan.completed_count();
            if current_completed == last_completed_count {
                no_progress_iterations += 1;
                if no_progress_iterations > 600 {
                    // 60 seconds of no progress (100 * 10ms sleep)
                    return Err(InstallError::NoProgress {
                        message: "No progress made in package installation for 60 seconds"
                            .to_string(),
                    }
                    .into());
                }
            } else {
                no_progress_iterations = 0;
                last_completed_count = current_completed;
            }
            // Try to start new tasks from ready queue
            while let Some(package_id) = ready_queue.pop() {
                context.emit_event(Event::DebugLog {
                    message: format!(
                        "DEBUG: Popped package {}-{} from ready queue",
                        package_id.name, package_id.version
                    ),
                    context: std::collections::HashMap::new(),
                });

                if inflight.contains_key(&package_id) {
                    context.emit_event(Event::DebugLog {
                        message: format!(
                            "DEBUG: Package {}-{} already in flight, skipping",
                            package_id.name, package_id.version
                        ),
                        context: std::collections::HashMap::new(),
                    });
                    continue; // Already in flight
                }

                let permit = self.resources.acquire_download_permit().await?;

                let node = resolved_packages.get(&package_id).ok_or_else(|| {
                    InstallError::PackageNotFound {
                        package: package_id.name.clone(),
                    }
                })?;

                context.emit_event(Event::DebugLog {
                    message: format!(
                        "DEBUG: Starting task for package {}-{} with action {:?}",
                        package_id.name, package_id.version, node.action
                    ),
                    context: std::collections::HashMap::new(),
                });

                let handle = self.spawn_package_task(
                    package_id.clone(),
                    node.clone(),
                    context.clone(),
                    permit,
                    prepared_packages.clone(),
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

        context.emit_event(Event::DebugLog {
            message: format!(
                "DEBUG: Exited main processing loop. execution_plan.is_complete()={}, inflight.is_empty()={}, prepared_packages.len()={}",
                execution_plan.is_complete(), inflight.is_empty(), prepared_packages.len()
            ),
            context: std::collections::HashMap::new(),
        });

        // Convert DashMap to HashMap and return prepared packages
        let prepared_packages =
            Arc::try_unwrap(prepared_packages).map_err(|_| InstallError::ConcurrencyError {
                message: "failed to unwrap prepared packages Arc".to_string(),
            })?;

        let mut result = HashMap::new();
        for entry in prepared_packages {
            result.insert(entry.0, entry.1);
        }
        Ok(result)
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
        prepared_packages: Arc<DashMap<PackageId, PreparedPackage>>,
    ) -> JoinHandle<Result<PackageId, Error>> {
        let net_client = self.net_client.clone();
        let store = self.store.clone();
        let state_manager = self.state_manager.clone();
        let timeout_duration = self.download_timeout;

        tokio::spawn(async move {
            Self::process_package(ProcessPackageArgs {
                package_id,
                node,
                context,
                net_client,
                store,
                state_manager,
                timeout_duration,
                prepared_packages,
            })
            .await
        })
    }

    /// Process a single package (download/local)
    async fn process_package(args: ProcessPackageArgs) -> Result<PackageId, Error> {
        let ProcessPackageArgs {
            package_id,
            node,
            context,
            net_client,
            store,
            state_manager: _state_manager,
            timeout_duration,
            prepared_packages,
        } = args;
        context.emit_event(Event::DebugLog {
            message: format!(
                "DEBUG: Processing package {}-{} with action {:?}",
                package_id.name, package_id.version, node.action
            ),
            context: std::collections::HashMap::new(),
        });

        context.emit_event(Event::PackageInstalling {
            name: package_id.name.clone(),
            version: package_id.version.clone(),
        });

        match node.action {
            NodeAction::Download => {
                if let Some(url) = &node.url {
                    // Download package with timeout and add to store (no validation)
                    let download_result = timeout(
                        timeout_duration,
                        Self::download_package_only(
                            url,
                            &package_id,
                            &net_client,
                            &store,
                            &context,
                            &prepared_packages,
                        ),
                    )
                    .await;

                    match download_result {
                        Ok(Ok(())) => {
                            context.emit_event(Event::PackageDownloaded {
                                name: package_id.name.clone(),
                                version: package_id.version.clone(),
                            });
                        }
                        Ok(Err(e)) => return Err(e),
                        Err(_) => {
                            return Err(InstallError::DownloadTimeout {
                                package: package_id.name.clone(),
                                url: url.to_string(),
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
                context.emit_event(Event::DebugLog {
                    message: format!(
                        "DEBUG: Processing local package {}-{}, path: {:?}",
                        package_id.name, package_id.version, node.path
                    ),
                    context: std::collections::HashMap::new(),
                });

                if let Some(path) = &node.path {
                    context.emit_event(Event::DebugLog {
                        message: format!(
                            "DEBUG: Adding local package to store: {}",
                            path.display()
                        ),
                        context: std::collections::HashMap::new(),
                    });

                    // For local packages, add to store and prepare data
                    let stored_package = store.add_package(path).await?;

                    if let Some(hash) = stored_package.hash() {
                        let size = stored_package.size().await?;
                        let store_path = stored_package.path().to_path_buf();

                        context.emit_event(Event::DebugLog {
                            message: format!(
                                "DEBUG: Local package stored with hash {} at {}",
                                hash.to_hex(),
                                store_path.display()
                            ),
                            context: std::collections::HashMap::new(),
                        });

                        let prepared_package = PreparedPackage {
                            hash: hash.clone(),
                            size,
                            store_path,
                            is_local: true,
                        };

                        prepared_packages.insert(package_id.clone(), prepared_package);

                        context.emit_event(Event::DebugLog {
                            message: format!(
                                "DEBUG: Added prepared package for {}-{}",
                                package_id.name, package_id.version
                            ),
                            context: std::collections::HashMap::new(),
                        });

                        context.emit_event(Event::PackageInstalled {
                            name: package_id.name.clone(),
                            version: package_id.version.clone(),
                            path: path.display().to_string(),
                        });
                    } else {
                        return Err(InstallError::AtomicOperationFailed {
                            message: "failed to get hash from local package".to_string(),
                        }
                        .into());
                    }
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

    /// Download a package and add to store (no validation - AtomicInstaller handles that)
    async fn download_package_only(
        url: &str,
        package_id: &PackageId,
        net_client: &NetClient,
        store: &PackageStore,
        context: &ExecutionContext,
        prepared_packages: &Arc<DashMap<PackageId, PreparedPackage>>,
    ) -> Result<(), Error> {
        // Download to temporary file first
        let temp_file =
            tempfile::NamedTempFile::new().map_err(|e| InstallError::TempFileError {
                message: e.to_string(),
            })?;

        // Download with progress reporting
        net_client
            .download_file_with_progress(url, temp_file.path(), |progress| {
                context.emit_event(Event::DownloadProgress {
                    url: url.to_string(),
                    bytes_downloaded: progress.downloaded,
                    total_bytes: progress.total,
                });
            })
            .await?;

        // Add to store and prepare package data
        let stored_package = store
            .add_package_from_file(temp_file.path(), &package_id.name, &package_id.version)
            .await?;

        // Prepare package data for AtomicInstaller (no database operations)
        if let Some(hash) = stored_package.hash() {
            let size = stored_package.size().await?;
            let store_path = stored_package.path().to_path_buf();

            let prepared_package = PreparedPackage {
                hash: hash.clone(),
                size,
                store_path,
                is_local: false,
            };

            prepared_packages.insert(package_id.clone(), prepared_package);

            context.emit_event(Event::DebugLog {
                message: format!(
                    "Package {}-{} downloaded and stored with hash {} (prepared for installation)",
                    package_id.name,
                    package_id.version,
                    hash.to_hex()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        Ok(())
    }

    /// Wait for at least one task to complete
    async fn wait_for_completion(
        &self,
        inflight: &DashMap<PackageId, JoinHandle<Result<PackageId, Error>>>,
    ) -> Result<PackageId, Error> {
        let timeout_duration = Duration::from_secs(300); // 5 minutes per task
        let start_time = Instant::now();

        loop {
            // Check if overall timeout exceeded
            if start_time.elapsed() > timeout_duration {
                return Err(InstallError::TaskError {
                    message: "Task completion timeout exceeded (5 minutes)".to_string(),
                }
                .into());
            }

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
                                message: format!("Task failed for {}: {e}", package_id.name),
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
}

impl EventEmitter for ExecutionContext {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Execution node for tracking dependencies
struct ExecutionNode {
    /// Action to perform (stored for future use in execution graph)
    #[allow(dead_code)]
    action: NodeAction,
    /// Remaining dependencies
    in_degree: AtomicUsize,
    /// Parent packages (for future dependency tracking, rollback, and error reporting)
    #[allow(dead_code)]
    parents: Vec<PackageId>,
}
