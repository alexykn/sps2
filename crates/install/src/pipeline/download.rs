//! Download pipeline stage implementation

use sps2_errors::{Error, InstallError};
use sps2_events::{EventSender, ProgressManager};
use sps2_hash::Hash;
use sps2_net::{PackageDownloadRequest, PackageDownloader};
use sps2_resolver::{ExecutionPlan, PackageId, ResolvedNode};
use sps2_resources::ResourceManager;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
    #[allow(dead_code)]
    resources: Arc<ResourceManager>,
    progress_manager: Arc<ProgressManager>,
    #[allow(dead_code)]
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
        self.progress_manager.register_child_tracker(
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
                        signature_url: node.signature_url.clone(),
                        expected_hash: node.expected_hash.clone(),
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
        self.progress_manager.complete_child_tracker(
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
}
