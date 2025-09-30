//! Main downloader orchestration and `PackageDownloader` implementation

use super::config::{
    DownloadResult, PackageDownloadConfig, PackageDownloadRequest, PackageDownloadResult,
    StreamParams,
};
use super::resume::get_resume_offset;
use super::retry::calculate_backoff_delay;
use super::stream::{download_file_simple, stream_download};
use super::validation::{validate_response, validate_url};
use crate::client::{NetClient, NetConfig};
use sps2_errors::{Error, NetworkError};
use sps2_events::{
    AppEvent, EventEmitter, EventSender, FailureContext, GeneralEvent, LifecycleEvent,
};
use sps2_hash::Hash;
use sps2_types::Version;
use std::path::Path;

use std::time::{Duration, Instant};
use tokio::fs as tokio_fs;

/// A streaming package downloader with resumable capabilities
pub struct PackageDownloader {
    config: PackageDownloadConfig,
    client: NetClient,
    progress_manager: sps2_events::ProgressManager,
}

impl PackageDownloader {
    /// Create a new package downloader
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be initialized.
    pub fn new(
        config: PackageDownloadConfig,
        progress_manager: sps2_events::ProgressManager,
    ) -> Result<Self, Error> {
        let net_config = NetConfig {
            timeout: Duration::from_secs(600), // 10 minutes for large files
            connect_timeout: Duration::from_secs(30),
            retry_count: config.retry_config.max_retries,
            retry_delay: config.retry_config.initial_delay,
            ..NetConfig::default()
        };

        let client = NetClient::new(net_config)?;

        Ok(Self {
            config,
            client,
            progress_manager,
        })
    }

    /// Create with default configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be initialized.
    pub fn with_defaults(progress_manager: sps2_events::ProgressManager) -> Result<Self, Error> {
        Self::new(PackageDownloadConfig::default(), progress_manager)
    }

    /// Download a .sp package file with concurrent signature download
    ///
    /// # Errors
    ///
    /// Returns an error if the download fails, hash verification fails,
    /// or file I/O operations fail.
    #[allow(clippy::too_many_arguments)] // Core download function requires all parameters for operation
    pub async fn download_package(
        &self,
        package_name: &str,
        version: &Version,
        package_url: &str,
        signature_url: Option<&str>,
        dest_dir: &Path,
        expected_hash: Option<&Hash>,
        progress_tracker_id: String,
        parent_progress_id: Option<String>,
        tx: &EventSender,
    ) -> Result<PackageDownloadResult, Error> {
        let start_time = Instant::now();

        // Create destination paths
        // Extract filename from URL instead of constructing it
        let package_filename = package_url
            .split('/')
            .next_back()
            .unwrap_or(&format!("{package_name}-{version}.sp"))
            .to_string();
        let package_path = dest_dir.join(&package_filename);
        let signature_path =
            signature_url.map(|_| dest_dir.join(format!("{package_filename}.minisig")));

        // Ensure destination directory exists
        tokio_fs::create_dir_all(dest_dir).await?;

        // Download package and signature concurrently
        // Create progress tracker if not provided
        let tracker_id = if progress_tracker_id.is_empty() {
            let config = sps2_events::patterns::DownloadProgressConfig {
                operation_name: format!("Downloading {package_name}"),
                total_bytes: None,
                package_name: Some(package_name.to_string()),
                url: package_url.to_string(),
            };
            self.progress_manager.create_download_tracker(&config)
        } else {
            progress_tracker_id
        };

        let package_fut = self.download_with_resume(
            package_url,
            &package_path,
            expected_hash,
            tracker_id,
            parent_progress_id.clone(),
            Some(package_name.to_string()),
            tx.clone(),
        );

        let signature_fut = async {
            match (signature_url, &signature_path) {
                (Some(sig_url), Some(sig_path)) => {
                    download_file_simple(&self.client, sig_url, sig_path, tx).await
                }
                _ => Ok(()),
            }
        };

        // Execute downloads concurrently
        let (package_result, _signature_result) = tokio::try_join!(package_fut, signature_fut)?;

        let download_time = start_time.elapsed();

        // Verify signature if available
        let signature_verified = if let Some(sig_path) = &signature_path {
            if sig_path.exists() {
                self.verify_package_signature(&package_path, sig_path)
                    .await?
            } else {
                false
            }
        } else {
            false
        };

        Ok(PackageDownloadResult {
            package_path,
            signature_path,
            hash: package_result.hash,
            size: package_result.size,
            download_time,
            signature_verified,
        })
    }

    /// Verify the signature of a downloaded package
    async fn verify_package_signature(
        &self,
        package_path: &Path,
        signature_path: &Path,
    ) -> Result<bool, NetworkError> {
        let sig_str = tokio::fs::read_to_string(signature_path)
            .await
            .map_err(|e| {
                NetworkError::DownloadFailed(format!(
                    "Failed to read signature file {}: {e}",
                    signature_path.display()
                ))
            })?;

        // For now, use the bootstrap/trusted keys loaded by ops during reposync
        // The allowed keys resolution is performed at a higher layer; here we emit result only
        // We do a best-effort verify via the shared helper using any available key files under KEYS_DIR
        let mut allowed = Vec::new();
        let keys_dir = std::path::Path::new(sps2_config::fixed_paths::KEYS_DIR);
        let keys_file = keys_dir.join("trusted_keys.json");
        if let Ok(content) = tokio::fs::read_to_string(&keys_file).await {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(obj) = json.as_object() {
                    for (key_id, entry) in obj {
                        if let Some(pk) = entry.get("public_key").and_then(|v| v.as_str()) {
                            allowed.push(sps2_signing::PublicKeyRef {
                                id: key_id.clone(),
                                algo: sps2_signing::Algorithm::Minisign,
                                data: pk.to_string(),
                            });
                        }
                    }
                }
            }
        }

        let verified = if allowed.is_empty() {
            false
        } else {
            sps2_signing::verify_minisign_file_with_keys(package_path, &sig_str, &allowed).is_ok()
        };

        Ok(verified)
    }

    /// Download multiple packages concurrently
    ///
    /// # Errors
    ///
    /// Returns an error if any download fails. Successful downloads are preserved.
    ///
    /// # Panics
    ///
    /// Panics if the semaphore is closed (which should not happen in normal operation).
    pub async fn download_packages_batch(
        &self,
        packages: Vec<PackageDownloadRequest>,
        dest_dir: &Path,
        batch_progress_id: Option<String>,
        tx: &EventSender,
    ) -> Result<Vec<PackageDownloadResult>, Error> {
        use futures::stream::{FuturesUnordered, StreamExt};

        let mut futures = FuturesUnordered::new();
        let total_packages = packages.len();

        for request in packages {
            let downloader = self.clone();
            let dest_dir = dest_dir.to_path_buf();
            let tx = tx.clone();
            let batch_progress_id_clone = batch_progress_id.clone();

            let fut = async move {
                let _permit = downloader
                    .config
                    .resources
                    .acquire_download_permit()
                    .await?;

                // Create individual progress tracker for this download
                let child_config = sps2_events::patterns::DownloadProgressConfig {
                    operation_name: format!("Downloading {}", request.name),
                    total_bytes: None, // Will be determined during download
                    package_name: Some(request.name.clone()),
                    url: request.package_url.clone(),
                };

                let child_id = downloader
                    .progress_manager
                    .create_download_tracker(&child_config);

                // Register as child of batch operation if we have a parent
                if let Some(ref parent_id) = batch_progress_id_clone {
                    #[allow(clippy::cast_precision_loss)]
                    // Acceptable precision loss for progress weights
                    let weight = 1.0 / total_packages as f64; // Equal weight for each package
                    downloader.progress_manager.register_child_tracker(
                        parent_id,
                        &child_id,
                        format!("Downloading {}", request.name),
                        weight,
                        &tx,
                    );
                }

                let result = downloader
                    .download_package(
                        &request.name,
                        &request.version,
                        &request.package_url,
                        request.signature_url.as_deref(),
                        &dest_dir,
                        request.expected_hash.as_ref(),
                        child_id.clone(), // Individual progress tracker ID
                        batch_progress_id_clone.clone(), // Parent progress ID
                        &tx,
                    )
                    .await;

                // Complete child tracker
                if let Some(ref parent_id) = batch_progress_id_clone {
                    let success = result.is_ok();
                    downloader
                        .progress_manager
                        .complete_child_tracker(parent_id, &child_id, success, &tx);
                }

                result
            };

            futures.push(fut);
        }

        let mut results = Vec::new();
        while let Some(result) = futures.next().await {
            results.push(result?);
        }

        Ok(results)
    }

    /// Download a file with resumable capability
    ///
    /// # Errors
    ///
    /// Returns an error if the download fails, network issues occur,
    /// hash verification fails, or file I/O operations fail.
    #[allow(clippy::too_many_arguments)]
    pub async fn download_with_resume(
        &self,
        url: &str,
        dest_path: &Path,
        expected_hash: Option<&Hash>,
        progress_tracker_id: String,
        parent_progress_id: Option<String>,
        package: Option<String>,
        tx: EventSender,
    ) -> Result<DownloadResult, Error> {
        let url = validate_url(url)?;
        let mut retry_count = 0;
        #[allow(unused_assignments)] // Used after retry loop for error reporting
        let mut last_error: Option<Error> = None;

        loop {
            match self
                .try_download_with_resume(
                    &url,
                    dest_path,
                    expected_hash,
                    progress_tracker_id.clone(),
                    parent_progress_id.clone(),
                    package.as_deref(),
                    &tx,
                )
                .await
            {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_error = Some(e);
                    retry_count += 1;

                    if retry_count > self.config.retry_config.max_retries {
                        break;
                    }

                    // Calculate backoff delay with jitter
                    let delay = calculate_backoff_delay(&self.config.retry_config, retry_count);

                    // Emit retry event with progress preservation
                    {
                        // Get current progress from partial download
                        let accumulated_bytes =
                            if let Ok(metadata) = tokio::fs::metadata(dest_path).await {
                                metadata.len()
                            } else {
                                0
                            };

                        tx.emit(AppEvent::Progress(sps2_events::ProgressEvent::Paused {
                            id: progress_tracker_id.clone(),
                            reason: format!(
                                "Retry attempt {}/{}",
                                retry_count, self.config.retry_config.max_retries
                            ),
                            items_completed: accumulated_bytes,
                        }));
                    }

                    tx.emit(AppEvent::General(GeneralEvent::DebugLog {
                        message: format!(
                            "Download failed, retrying in {delay:?} (attempt {retry_count}/{})...",
                            self.config.retry_config.max_retries
                        ),
                        context: std::collections::HashMap::new(),
                    }));

                    tokio::time::sleep(delay).await;

                    // Resume progress tracking
                    tx.emit(AppEvent::Progress(sps2_events::ProgressEvent::Resumed {
                        id: progress_tracker_id.clone(),
                        pause_duration: delay,
                    }));
                }
            }
        }

        let final_error = last_error.unwrap_or_else(|| {
            NetworkError::DownloadFailed("Maximum retries exceeded".to_string()).into()
        });

        let failure = FailureContext::from_error(&final_error);

        tx.emit(AppEvent::Lifecycle(LifecycleEvent::download_failed(
            url.to_string(),
            package.clone(),
            failure,
        )));

        Err(final_error)
    }

    /// Attempt a single download with resume capability
    #[allow(clippy::too_many_arguments)]
    async fn try_download_with_resume(
        &self,
        url: &str,
        dest_path: &Path,
        expected_hash: Option<&Hash>,
        progress_tracker_id: String,
        parent_progress_id: Option<String>,
        package: Option<&str>,
        tx: &EventSender,
    ) -> Result<DownloadResult, Error> {
        // Check if partial file exists
        let resume_offset = get_resume_offset(&self.config, dest_path).await?;

        // Prepare request with range header if resuming
        let mut headers = Vec::new();
        if resume_offset > 0 {
            headers.push(("Range", format!("bytes={resume_offset}-")));
        }

        // Make HTTP request
        let response = if headers.is_empty() {
            self.client.get(url).await?
        } else {
            self.client
                .get_with_headers(
                    url,
                    &headers
                        .iter()
                        .map(|(k, v)| (*k, v.as_str()))
                        .collect::<Vec<_>>(),
                )
                .await?
        };

        // Validate response
        validate_response(&response, resume_offset > 0)?;

        // Get total size information
        let content_length = response.content_length().unwrap_or(0);
        let total_size =
            if resume_offset > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                // For partial content, content-length is the remaining bytes
                resume_offset + content_length
            } else {
                content_length
            };

        // Validate file size limits
        if total_size > self.config.max_file_size {
            return Err(NetworkError::FileSizeExceeded {
                size: total_size,
                limit: self.config.max_file_size,
            }
            .into());
        }

        tx.emit(AppEvent::Lifecycle(LifecycleEvent::download_started(
            url.to_string(),
            package.map(str::to_string),
            Some(total_size),
        )));

        // Download with streaming and progress
        let params = StreamParams {
            total_size,
            expected_hash,
            event_sender: tx,
            url,
            progress_tracker_id,
            parent_progress_id,
            progress_manager: Some(&self.progress_manager),
        };
        let result =
            stream_download(&self.config, response, dest_path, resume_offset, &params).await?;

        tx.emit(AppEvent::Lifecycle(LifecycleEvent::download_completed(
            url.to_string(),
            package.map(str::to_string),
            result.size,
        )));

        Ok(result)
    }
}

impl Clone for PackageDownloader {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            client: self.client.clone(),
            progress_manager: self.progress_manager.clone(),
        }
    }
}
