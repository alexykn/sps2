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
use sps2_events::{Event, EventSender, EventSenderExt};
use sps2_hash::Hash;
use sps2_types::Version;
use std::path::Path;

use std::time::{Duration, Instant};
use tokio::fs as tokio_fs;


/// A streaming package downloader with resumable capabilities
pub struct PackageDownloader {
    config: PackageDownloadConfig,
    client: NetClient,
}

impl PackageDownloader {
    /// Create a new package downloader
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be initialized.
    pub fn new(config: PackageDownloadConfig) -> Result<Self, Error> {
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
        })
    }

    /// Create with default configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be initialized.
    pub fn with_defaults() -> Result<Self, Error> {
        Self::new(PackageDownloadConfig::default())
    }

    /// Download a .sp package file with concurrent signature download
    ///
    /// # Errors
    ///
    /// Returns an error if the download fails, hash verification fails,
    /// or file I/O operations fail.
    #[allow(clippy::too_many_arguments)]
    pub async fn download_package(
        &self,
        package_name: &str,
        version: &Version,
        package_url: &str,
        signature_url: Option<&str>,
        dest_dir: &Path,
        expected_hash: Option<&Hash>,
        tx: &EventSender,
    ) -> Result<PackageDownloadResult, Error> {
        let start_time = Instant::now();

        tx.emit(Event::PackageDownloadStarted {
            name: package_name.to_string(),
            version: version.clone(),
            url: package_url.to_string(),
        });

        // Create destination paths
        let package_filename = format!("{package_name}-{version}.sp");
        let package_path = dest_dir.join(&package_filename);
        let signature_path =
            signature_url.map(|_| dest_dir.join(format!("{package_filename}.minisig")));

        // Ensure destination directory exists
        tokio_fs::create_dir_all(dest_dir).await?;

        // Download package and signature concurrently
        let package_fut =
            self.download_with_resume(package_url, &package_path, expected_hash, tx.clone());

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

        tx.emit(Event::PackageDownloaded {
            name: package_name.to_string(),
            version: version.clone(),
        });

        // Verify signature if available
        let signature_verified = if signature_path.is_some() {
            // TODO: Implement minisign verification once crypto support is added
            tx.emit(Event::PackageSignatureDownloaded {
                name: package_name.to_string(),
                version: version.clone(),
                verified: false, // Placeholder until minisign verification is implemented
            });
            false
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
        tx: &EventSender,
    ) -> Result<Vec<PackageDownloadResult>, Error> {
        use futures::stream::{FuturesUnordered, StreamExt};

        let mut futures = FuturesUnordered::new();

        for request in packages {
            let downloader = self.clone();
            let dest_dir = dest_dir.to_path_buf();
            let tx = tx.clone();

            let fut = async move {
                let _permit = downloader.config.resources.acquire_download_permit().await?;
                downloader
                    .download_package(
                        &request.name,
                        &request.version,
                        &request.package_url,
                        request.signature_url.as_deref(),
                        &dest_dir,
                        request.expected_hash.as_ref(),
                        &tx,
                    )
                    .await
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
    pub async fn download_with_resume(
        &self,
        url: &str,
        dest_path: &Path,
        expected_hash: Option<&Hash>,
        tx: EventSender,
    ) -> Result<DownloadResult, Error> {
        let url = validate_url(url)?;
        let mut retry_count = 0;
        #[allow(unused_assignments)]
        let mut last_error: Option<Error> = None;

        loop {
            match self
                .try_download_with_resume(&url, dest_path, expected_hash, &tx)
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

                    tx.emit(Event::DebugLog {
                        message: format!(
                            "Download failed, retrying in {delay:?} (attempt {retry_count}/{})...",
                            self.config.retry_config.max_retries
                        ),
                        context: std::collections::HashMap::new(),
                    });

                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            NetworkError::DownloadFailed("Maximum retries exceeded".to_string()).into()
        }))
    }

    /// Attempt a single download with resume capability
    async fn try_download_with_resume(
        &self,
        url: &str,
        dest_path: &Path,
        expected_hash: Option<&Hash>,
        tx: &EventSender,
    ) -> Result<DownloadResult, Error> {
        // Check if partial file exists
        let resume_offset = get_resume_offset(&self.config, dest_path).await?;

        if resume_offset > 0 {
            tx.emit(Event::DownloadResuming {
                url: url.to_string(),
                offset: resume_offset,
                total_size: None,
            });
        }

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

        tx.emit(Event::DownloadStarted {
            url: url.to_string(),
            size: Some(total_size),
        });

        // Download with streaming and progress
        let params = StreamParams {
            total_size,
            expected_hash,
            event_sender: tx,
            url,
        };
        let result =
            stream_download(&self.config, response, dest_path, resume_offset, &params).await?;

        tx.emit(Event::DownloadCompleted {
            url: url.to_string(),
            size: result.size,
        });

        Ok(result)
    }
}

impl Clone for PackageDownloader {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            client: self.client.clone(),
        }
    }
}
