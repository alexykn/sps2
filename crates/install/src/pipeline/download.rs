//! Download pipeline stage implementation

use crossbeam::queue::SegQueue;
use dashmap::DashMap;
use sps2_errors::{Error, InstallError};
use sps2_events::{EventSender, ProgressManager};
use sps2_hash::Hash;
use sps2_net::PackageDownloader;
use sps2_resolver::{ExecutionPlan, NodeAction, PackageId, ResolvedNode};
use sps2_resources::ResourceManager;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Result of package download operation
#[derive(Debug)]
pub struct DownloadResult {
    pub package_id: PackageId,
    pub downloaded_path: PathBuf,
    pub hash: Hash,
    #[allow(dead_code)] // Held for automatic cleanup on drop - TODO: redesign ownership model
    pub temp_dir: Option<tempfile::TempDir>,
    pub node: ResolvedNode,
}

/// Download pipeline stage coordinator
pub struct DownloadPipeline {
    downloader: PackageDownloader,
    resources: Arc<ResourceManager>,
    progress_manager: Arc<ProgressManager>,
    operation_timeout: Duration,
}

impl DownloadPipeline {
    /// Create a new download pipeline
    pub fn new(
        downloader: PackageDownloader,
        resources: Arc<ResourceManager>,
        progress_manager: Arc<ProgressManager>,
        operation_timeout: Duration,
    ) -> Self {
        Self {
            downloader,
            resources,
            progress_manager,
            operation_timeout,
        }
    }

    /// Execute parallel downloads with dependency ordering
    pub async fn execute_parallel_downloads(
        &self,
        execution_plan: &ExecutionPlan,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
        progress_id: &str,
        tx: &EventSender,
    ) -> Result<Vec<DownloadResult>, Error> {
        let mut results = Vec::new();
        let ready_queue = Arc::new(SegQueue::new());
        let completed = Arc::new(DashMap::<PackageId, bool>::new());

        // Initialize with packages that have no dependencies
        for package_id in execution_plan.ready_packages() {
            ready_queue.push(package_id);
        }

        // Process downloads with dependency ordering
        while completed.len() < resolved_packages.len() {
            // Start downloads for ready packages
            let mut handles = Vec::new();

            while let Some(package_id) = ready_queue.pop() {
                if completed.contains_key(&package_id) {
                    continue;
                }

                if let Some(node) = resolved_packages.get(&package_id) {
                    let handle = self.spawn_download_task(
                        package_id.clone(),
                        node.clone(),
                        progress_id.to_string(),
                        tx.clone(),
                    );
                    handles.push((package_id, handle));
                }
            }

            // Wait for at least one download to complete
            if !handles.is_empty() {
                let (completed_package, result) =
                    self.wait_for_download_completion(handles).await?;
                completed.insert(completed_package.clone(), true);
                results.push(result);

                // Check if new packages became ready
                let newly_ready = execution_plan.complete_package(&completed_package);
                for pkg in newly_ready {
                    ready_queue.push(pkg);
                }

                // Update progress
                self.progress_manager.update_progress(
                    progress_id,
                    completed.len() as u64,
                    Some(resolved_packages.len() as u64),
                    tx,
                );
            }

            // Small delay to prevent busy waiting
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(results)
    }

    /// Spawn a download task for a single package
    pub fn spawn_download_task(
        &self,
        package_id: PackageId,
        node: ResolvedNode,
        _progress_id: String,
        tx: EventSender,
    ) -> JoinHandle<Result<DownloadResult, Error>> {
        let downloader = self.downloader.clone();
        let resources = self.resources.clone();
        let timeout = self.operation_timeout;

        tokio::spawn(async move {
            let _permit = resources.acquire_download_permit().await?;

            let estimated_size = 50 * 1024 * 1024; // Estimate 50MB per download
            if resources.limits.memory_usage.is_some() {
                resources
                    .memory_usage
                    .fetch_add(estimated_size, Ordering::Relaxed);
            }

            let result = tokio::time::timeout(
                timeout,
                Self::download_package_with_progress(&downloader, &package_id, &node, &tx),
            )
            .await;

            if resources.limits.memory_usage.is_some() {
                resources
                    .memory_usage
                    .fetch_sub(estimated_size, Ordering::Relaxed);
            }

            match result {
                Ok(Ok(download_result)) => Ok(download_result),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(InstallError::DownloadTimeout {
                    package: package_id.name,
                    url: node.url.unwrap_or_default(),
                    timeout_seconds: timeout.as_secs(),
                }
                .into()),
            }
        })
    }

    /// Download a package with progress reporting
    async fn download_package_with_progress(
        downloader: &PackageDownloader,
        package_id: &PackageId,
        node: &ResolvedNode,
        tx: &EventSender,
    ) -> Result<DownloadResult, Error> {
        match &node.action {
            NodeAction::Download => {
                if let Some(url) = &node.url {
                    let temp_dir =
                        tempfile::tempdir().map_err(|e| InstallError::TempFileError {
                            message: format!("failed to create temp dir: {e}"),
                        })?;

                    let result = downloader
                        .download_package(
                            &package_id.name,
                            &package_id.version,
                            url,
                            None, // No signature URL for now
                            temp_dir.path(),
                            None, // No expected hash for now
                            tx,
                        )
                        .await?;

                    Ok(DownloadResult {
                        package_id: package_id.clone(),
                        downloaded_path: result.package_path,
                        hash: result.hash,
                        temp_dir: Some(temp_dir),
                        node: node.clone(),
                    })
                } else {
                    Err(InstallError::MissingDownloadUrl {
                        package: package_id.name.clone(),
                    }
                    .into())
                }
            }
            NodeAction::Local => {
                if let Some(path) = &node.path {
                    // Compute hash for local file
                    let hash = Hash::hash_file(path).await?;

                    Ok(DownloadResult {
                        package_id: package_id.clone(),
                        downloaded_path: path.clone(),
                        hash,
                        temp_dir: None,
                        node: node.clone(),
                    })
                } else {
                    Err(InstallError::MissingLocalPath {
                        package: package_id.name.clone(),
                    }
                    .into())
                }
            }
        }
    }

    /// Wait for at least one download to complete
    async fn wait_for_download_completion(
        &self,
        mut handles: Vec<(PackageId, JoinHandle<Result<DownloadResult, Error>>)>,
    ) -> Result<(PackageId, DownloadResult), Error> {
        loop {
            for i in (0..handles.len()).rev() {
                if handles[i].1.is_finished() {
                    let (package_id, handle) = handles.remove(i);
                    match handle.await {
                        Ok(Ok(result)) => return Ok((package_id, result)),
                        Ok(Err(e)) => return Err(e),
                        Err(e) => {
                            return Err(InstallError::TaskError {
                                message: format!("download task failed: {e}"),
                            }
                            .into())
                        }
                    }
                }
            }

            // Small delay before checking again
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
