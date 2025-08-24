//! Download pipeline stage implementation

use sps2_errors::{Error, InstallError};
use sps2_events::{patterns::DownloadProgressConfig, EventSender, ProgressManager};
use sps2_hash::Hash;
use sps2_net::{PackageDownloadRequest, PackageDownloader};
use sps2_resolver::{ExecutionPlan, NodeAction, PackageId, ResolvedNode};
use sps2_resources::ResourceManager;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use sps2_events::EventEmitter;

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
    #[allow(dead_code)] // Reserved for future resource management features
    resources: Arc<ResourceManager>,
    progress_manager: Arc<ProgressManager>,
    #[allow(dead_code)] // Reserved for future timeout handling features
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
        _execution_plan: &ExecutionPlan,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
        parent_progress_id: &str,
        tx: &EventSender,
    ) -> Result<Vec<DownloadResult>, Error> {
        // Create batch progress tracker for downloads phase
        let download_config = sps2_events::patterns::DownloadProgressConfig {
            operation_name: format!("Downloading {} packages", resolved_packages.len()),
            total_bytes: None,
            package_name: None,
            url: "batch".to_string(),
        };

        let batch_progress_id = self
            .progress_manager
            .create_download_tracker(&download_config);

        // Register as child of parent operation (e.g., install operation)
        let _ = self.progress_manager.register_child_tracker(
            parent_progress_id,
            &batch_progress_id,
            "Download Phase".to_string(),
            0.5, // Downloads are 50% of install operation
            tx,
        );

        // Convert resolved packages to download requests
        let download_requests: Vec<PackageDownloadRequest> = resolved_packages
            .iter()
            .filter_map(|(package_id, node)| {
                if let Some(url) = &node.url {
                    Some(PackageDownloadRequest {
                        name: package_id.name.clone(),
                        version: package_id.version.clone(),
                        package_url: url.clone(),
                        signature_url: None,
                        expected_hash: None,
                    })
                } else {
                    None
                }
            })
            .collect();

        let temp_dir = tempfile::tempdir().map_err(|e| InstallError::TempFileError {
            message: format!("failed to create temp dir: {e}"),
        })?;

        // Use the updated batch download method
        let download_results = self
            .downloader
            .download_packages_batch(
                download_requests,
                temp_dir.path(),
                Some(batch_progress_id.clone()),
                tx,
            )
            .await?;

        // Complete batch progress
        let _ = self.progress_manager.complete_child_tracker(
            parent_progress_id,
            &batch_progress_id,
            true,
            tx,
        );

        // Convert to DownloadResult format
        let mut results = Vec::new();
        for (package_download_result, (package_id, node)) in
            download_results.iter().zip(resolved_packages.iter())
        {
            results.push(DownloadResult {
                package_id: package_id.clone(),
                downloaded_path: package_download_result.package_path.clone(),
                hash: package_download_result.hash.clone(),
                temp_dir: None, // Temp dir ownership handled by batch operation
                node: node.clone(),
            });
        }

        Ok(results)
    }

    /// Spawn a download task for a single package
    #[allow(dead_code)] // Legacy method kept for compatibility
    pub fn spawn_download_task(
        &self,
        package_id: PackageId,
        node: ResolvedNode,
        _progress_id: String,
        tx: EventSender,
    ) -> JoinHandle<Result<DownloadResult, Error>> {
        let downloader = self.downloader.clone();
        let resources = self.resources.clone();
        let progress_manager = self.progress_manager.clone();
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
                Self::download_package_with_progress(
                    &downloader,
                    &package_id,
                    &node,
                    &progress_manager,
                    &tx,
                ),
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
    #[allow(dead_code)] // Legacy method kept for compatibility
    async fn download_package_with_progress(
        downloader: &PackageDownloader,
        package_id: &PackageId,
        node: &ResolvedNode,
        progress_manager: &ProgressManager,
        tx: &EventSender,
    ) -> Result<DownloadResult, Error> {
        match &node.action {
            NodeAction::Download => {
                if let Some(url) = &node.url {
                    let temp_dir =
                        tempfile::tempdir().map_err(|e| InstallError::TempFileError {
                            message: format!("failed to create temp dir: {e}"),
                        })?;

                    tx.emit(sps2_events::AppEvent::Download(
                        sps2_events::DownloadEvent::PackageStarted {
                            name: package_id.name.clone(),
                            version: package_id.version.clone(),
                            url: url.clone(),
                        },
                    ));

                    let download_config = DownloadProgressConfig {
                        operation_name: format!("Downloading {}", package_id.name),
                        total_bytes: None, // We don't know the total size yet
                        package_name: Some(package_id.name.clone()),
                        url: url.clone(),
                    };
                    let progress_id = progress_manager.create_download_tracker(&download_config);

                    let result = downloader
                        .download_package(
                            &package_id.name,
                            &package_id.version,
                            url,
                            None, // No signature URL for now
                            temp_dir.path(),
                            None, // No expected hash for now
                            progress_id.clone(),
                            None, // No parent progress ID
                            tx,
                        )
                        .await?;

                    progress_manager.complete_operation(&progress_id, tx);

                    tx.emit(sps2_events::AppEvent::Download(
                        sps2_events::DownloadEvent::PackageCompleted {
                            name: package_id.name.clone(),
                            version: package_id.version.clone(),
                        },
                    ));

                    // Copy the downloaded file to a persistent location to prevent temp dir cleanup
                    let persistent_dir = std::env::temp_dir().join("sps2_install_cache");
                    tokio::fs::create_dir_all(&persistent_dir)
                        .await
                        .map_err(|e| InstallError::TempFileError {
                            message: format!("failed to create persistent cache directory: {e}"),
                        })?;

                    let persistent_path =
                        persistent_dir.join(result.package_path.file_name().unwrap());

                    // Debug: print paths
                    eprintln!("DEBUG: Original path: {}", result.package_path.display());
                    eprintln!("DEBUG: Persistent path: {}", persistent_path.display());

                    tokio::fs::copy(&result.package_path, &persistent_path)
                        .await
                        .map_err(|e| InstallError::TempFileError {
                            message: format!(
                                "failed to copy downloaded file to persistent cache: {e}"
                            ),
                        })?;

                    // Verify the copy worked
                    if !persistent_path.exists() {
                        return Err(InstallError::TempFileError {
                            message: format!(
                                "Persistent file does not exist after copy: {}",
                                persistent_path.display()
                            ),
                        }
                        .into());
                    }

                    eprintln!("DEBUG: File copied successfully to persistent location");

                    Ok(DownloadResult {
                        package_id: package_id.clone(),
                        downloaded_path: persistent_path,
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
    #[allow(dead_code)] // Legacy method kept for compatibility
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
