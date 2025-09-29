//! Parallel executor for package operations

use crate::PreparedPackage;
use crossbeam::queue::SegQueue;
use dashmap::DashMap;
use sps2_errors::{Error, InstallError};
use sps2_events::{AppEvent, EventEmitter, GeneralEvent};
use sps2_resolver::{ExecutionPlan, NodeAction, PackageId, ResolvedNode};
use sps2_resources::ResourceManager;
use sps2_state::StateManager;
use sps2_store::PackageStore;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};

use super::context::ExecutionContext;
use super::worker::{process_package, ProcessPackageArgs};

/// Parallel executor for package operations
pub struct ParallelExecutor {
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
        context.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Execution plan has {} ready packages",
                execution_plan.ready_packages().len()
            ),
            context: std::collections::HashMap::from([
                (
                    "ready_packages".to_string(),
                    execution_plan
                        .ready_packages()
                        .iter()
                        .map(|id| format!("{}-{}", id.name, id.version))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ]),
        }));

        for package_id in execution_plan.ready_packages() {
            context.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: format!(
                    "DEBUG: Processing ready package {}-{}",
                    package_id.name, package_id.version
                ),
                context: std::collections::HashMap::new(),
            }));

            // Only add packages with in_degree 0 from our graph
            if let Some(node) = graph.get(&package_id) {
                let in_degree = node.in_degree.load(std::sync::atomic::Ordering::Relaxed);
                context.emit(AppEvent::General(GeneralEvent::DebugLog {
                    message: format!(
                        "DEBUG: Package {}-{} has in_degree {}",
                        package_id.name, package_id.version, in_degree
                    ),
                    context: std::collections::HashMap::new(),
                }));

                if in_degree == 0 {
                    ready_queue.push(package_id.clone());
                    context.emit(AppEvent::General(GeneralEvent::DebugLog {
                        message: format!(
                            "DEBUG: Added package {}-{} to ready queue",
                            package_id.name, package_id.version
                        ),
                        context: std::collections::HashMap::new(),
                    }));
                }
            } else {
                ready_queue.push(package_id.clone());
                context.emit(AppEvent::General(GeneralEvent::DebugLog {
                    message: format!(
                        "DEBUG: Added package {}-{} to ready queue (not in graph)",
                        package_id.name, package_id.version
                    ),
                    context: std::collections::HashMap::new(),
                }));
            }
        }

        // Process packages until completion with overall timeout
        let overall_timeout = Duration::from_secs(1800); // 30 minutes total
        let start_time = Instant::now();
        let mut no_progress_iterations = 0;
        let mut last_completed_count = 0;

        context.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Starting main processing loop. execution_plan.is_complete()={}, inflight.is_empty()={}",
                execution_plan.is_complete(), inflight.is_empty()
            ),
            context: std::collections::HashMap::new(),
        }));

        // Process packages until completion - ensure we process ready packages even if execution_plan reports complete
        while (!execution_plan.is_complete() || !inflight.is_empty()) || !ready_queue.is_empty() {
            context.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: format!(
                    "DEBUG: Loop iteration. execution_plan.is_complete()={}, inflight.is_empty()={}",
                    execution_plan.is_complete(), inflight.is_empty()
                ),
                context: std::collections::HashMap::new(),
            }));
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
                context.emit(AppEvent::General(GeneralEvent::DebugLog {
                    message: format!(
                        "DEBUG: Popped package {}-{} from ready queue",
                        package_id.name, package_id.version
                    ),
                    context: std::collections::HashMap::new(),
                }));

                if inflight.contains_key(&package_id) {
                    context.emit(AppEvent::General(GeneralEvent::DebugLog {
                        message: format!(
                            "DEBUG: Package {}-{} already in flight, skipping",
                            package_id.name, package_id.version
                        ),
                        context: std::collections::HashMap::new(),
                    }));
                    continue; // Already in flight
                }

                let permit = self.resources.acquire_download_permit().await?;

                let node = resolved_packages.get(&package_id).ok_or_else(|| {
                    InstallError::PackageNotFound {
                        package: package_id.name.clone(),
                    }
                })?;

                context.emit(AppEvent::General(GeneralEvent::DebugLog {
                    message: format!(
                        "DEBUG: Starting task for package {}-{} with action {:?}",
                        package_id.name, package_id.version, node.action
                    ),
                    context: std::collections::HashMap::new(),
                }));

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

        context.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Exited main processing loop. execution_plan.is_complete()={}, inflight.is_empty()={}, prepared_packages.len()={}",
                execution_plan.is_complete(), inflight.is_empty(), prepared_packages.len()
            ),
            context: std::collections::HashMap::new(),
        }));

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
        permit: tokio::sync::OwnedSemaphorePermit,
        prepared_packages: Arc<DashMap<PackageId, PreparedPackage>>,
    ) -> JoinHandle<Result<PackageId, Error>> {
        let store = self.store.clone();
        let state_manager = self.state_manager.clone();
        let timeout_duration = self.download_timeout;

        tokio::spawn(async move {
            process_package(ProcessPackageArgs {
                package_id,
                node,
                context,
                store,
                state_manager,
                timeout_duration,
                prepared_packages,
                permit,
            })
            .await
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_events::events::{AcquisitionEvent, AcquisitionSource};
    use sps2_events::{AppEvent, InstallEvent};
    use sps2_hash::{Hash as PackageHash, HashAlgorithm};
    use sps2_resolver::{DependencyGraph, ResolvedNode};
    use sps2_store::{create_package, PackageStore};
    use sps2_types::{Arch, Manifest, Version};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::fs as afs;

    use crate::prepare::context::ExecutionContext;
    use crate::prepare::worker::try_prepare_from_store;

    async fn mk_env() -> (TempDir, StateManager, PackageStore) {
        let td = TempDir::new().expect("tempdir");
        let state = StateManager::new(td.path()).await.expect("state manager");
        let store_base = td.path().join("store");
        afs::create_dir_all(&store_base).await.expect("store dir");
        let store = PackageStore::new(store_base);
        (td, state, store)
    }

    async fn create_sp(name: &str, version: &str) -> (TempDir, std::path::PathBuf) {
        let td = TempDir::new().expect("package dir");
        let src = td.path().join("src");
        afs::create_dir_all(&src).await.expect("src dir");

        let version = Version::parse(version).expect("valid version");
        let manifest = Manifest::new(name.to_string(), &version, 1, &Arch::Arm64);
        let manifest_path = src.join("manifest.toml");
        sps2_store::manifest_io::write_manifest(&manifest_path, &manifest)
            .await
            .expect("write manifest");

        let content_path = src.join("opt/pm/live/share");
        afs::create_dir_all(&content_path)
            .await
            .expect("content dir");
        afs::write(content_path.join("file.txt"), name.as_bytes())
            .await
            .expect("write file");

        let sp_path = td.path().join("pkg.sp");
        create_package(&src, &sp_path)
            .await
            .expect("create package");

        (td, sp_path)
    }

    #[tokio::test]
    async fn download_permit_limits_parallelism() {
        let (_td, state, store) = mk_env().await;

        let (_pkg1_dir, pkg1_sp) = create_sp("pkg-a", "1.0.0").await;
        let (_pkg2_dir, pkg2_sp) = create_sp("pkg-b", "1.0.0").await;

        let node1 = ResolvedNode::local(
            "pkg-a".to_string(),
            Version::parse("1.0.0").unwrap(),
            pkg1_sp.clone(),
            vec![],
        );
        let node2 = ResolvedNode::local(
            "pkg-b".to_string(),
            Version::parse("1.0.0").unwrap(),
            pkg2_sp.clone(),
            vec![],
        );

        let pkg1_id = node1.package_id();
        let pkg2_id = node2.package_id();

        let mut resolved_packages = HashMap::new();
        resolved_packages.insert(pkg1_id.clone(), node1.clone());
        resolved_packages.insert(pkg2_id.clone(), node2.clone());

        let mut graph = DependencyGraph::new();
        graph.add_node(node1);
        graph.add_node(node2);

        let sorted = vec![pkg1_id.clone(), pkg2_id.clone()];
        let execution_plan = ExecutionPlan::from_sorted_packages(&sorted, &graph);

        let limits = sps2_resources::limits::ResourceLimits {
            concurrent_downloads: 1,
            concurrent_decompressions: 1,
            concurrent_installations: 1,
            memory_usage: None,
        };
        let resources = Arc::new(sps2_resources::ResourceManager::new(limits));
        let executor = ParallelExecutor::new(store, state, resources).expect("parallel executor");

        let (tx, mut rx) = sps2_events::channel();
        let context = ExecutionContext::new().with_event_sender(tx);

        executor
            .execute_parallel(&execution_plan, &resolved_packages, &context)
            .await
            .expect("execute parallel");

        let mut sequence = Vec::new();
        while let Ok(message) = rx.try_recv() {
            if let AppEvent::Install(install_event) = message.event {
                match install_event {
                    InstallEvent::Started { package, .. } => {
                        sequence.push(("start", package));
                    }
                    InstallEvent::Completed { package, .. } => {
                        sequence.push(("complete", package));
                    }
                    InstallEvent::Failed { .. } => {}
                }
            }
        }

        let starts: Vec<_> = sequence
            .iter()
            .enumerate()
            .filter(|(_, (kind, _))| *kind == "start")
            .collect();
        let completes: Vec<_> = sequence
            .iter()
            .enumerate()
            .filter(|(_, (kind, _))| *kind == "complete")
            .collect();

        assert_eq!(starts.len(), 2, "expected two start events");
        assert_eq!(completes.len(), 2, "expected two completion events");
        assert!(
            starts[0].0 < completes[0].0,
            "first completion must follow first start"
        );
        assert!(
            completes[0].0 < starts[1].0,
            "second package should only start after first completes"
        );
    }

    #[tokio::test]
    async fn try_prepare_from_store_returns_package_when_available() {
        let (_td, state, store) = mk_env().await;
        let (_pkg_dir, pkg_sp) = create_sp("pkg-cache", "1.0.0").await;

        let stored_package = store.add_package(&pkg_sp).await.expect("store package");
        let store_hash = stored_package.hash().expect("hash");
        let expected_size = stored_package.size().await.expect("size");
        let package_hash = PackageHash::hash_file_with_algorithm(&pkg_sp, HashAlgorithm::Blake3)
            .await
            .expect("package hash");

        state
            .ensure_store_ref(&store_hash.to_hex(), expected_size as i64)
            .await
            .expect("store ref");

        state
            .add_package_map(
                "pkg-cache",
                "1.0.0",
                &store_hash.to_hex(),
                Some(&package_hash.to_hex()),
            )
            .await
            .expect("package map insert");

        let mut node = ResolvedNode::download(
            "pkg-cache".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://example.invalid/pkg-cache.sp".to_string(),
            vec![],
        );
        node.expected_hash = Some(package_hash.clone());

        let pkg_id = node.package_id();
        let prepared_packages = Arc::new(DashMap::new());
        let (tx, mut rx) = sps2_events::channel();
        let context = ExecutionContext::new().with_event_sender(tx);

        let size = try_prepare_from_store(
            &pkg_id,
            &node,
            &store,
            &state,
            &context,
            &prepared_packages,
        )
        .await
        .expect("reuse succeeds")
        .expect("should reuse store package");

        assert_eq!(size, expected_size);

        let entry = prepared_packages
            .get(&pkg_id)
            .expect("prepared package present");
        assert_eq!(entry.hash, store_hash);
        assert_eq!(entry.size, expected_size);
        assert!(!entry.is_local);
        assert_eq!(entry.package_hash.as_ref(), Some(&package_hash));
        drop(entry);

        let mut saw_store_acquisition = false;
        while let Ok(message) = rx.try_recv() {
            if let AppEvent::Acquisition(acq) = message.event {
                if matches!(
                    acq,
                    AcquisitionEvent::Completed {
                        source: AcquisitionSource::StoreCache { .. },
                        ..
                    }
                ) {
                    saw_store_acquisition = true;
                }
            }
        }
        assert!(saw_store_acquisition, "expected store acquisition event");
    }

    #[tokio::test]
    async fn try_prepare_from_store_respects_force_download() {
        let (_td, state, store) = mk_env().await;
        let (_pkg_dir, pkg_sp) = create_sp("pkg-force", "1.0.0").await;

        let stored_package = store.add_package(&pkg_sp).await.expect("store package");
        let store_hash = stored_package.hash().expect("hash");
        let package_hash = PackageHash::hash_file_with_algorithm(&pkg_sp, HashAlgorithm::Blake3)
            .await
            .expect("package hash");

        state
            .ensure_store_ref(
                &store_hash.to_hex(),
                stored_package.size().await.expect("size") as i64,
            )
            .await
            .expect("store ref");

        state
            .add_package_map(
                "pkg-force",
                "1.0.0",
                &store_hash.to_hex(),
                Some(&package_hash.to_hex()),
            )
            .await
            .expect("package map insert");

        let mut node = ResolvedNode::download(
            "pkg-force".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://example.invalid/pkg-force.sp".to_string(),
            vec![],
        );
        node.expected_hash = Some(package_hash);

        let pkg_id = node.package_id();
        let prepared_packages = Arc::new(DashMap::new());
        let (tx, _rx) = sps2_events::channel();
        let context = ExecutionContext::new()
            .with_event_sender(tx)
            .with_force_redownload(true);

        let result = try_prepare_from_store(
            &pkg_id,
            &node,
            &store,
            &state,
            &context,
            &prepared_packages,
        )
        .await
        .expect("call succeeds");

        assert!(result.is_none(), "expected force download to skip reuse");
        assert!(prepared_packages.is_empty());
    }
}