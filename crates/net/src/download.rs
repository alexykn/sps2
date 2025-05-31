//! File download with progress reporting and verification

use futures::StreamExt;
use sps2_errors::{Error, NetworkError};
use sps2_events::{Event, EventSender, EventSenderExt};
use sps2_hash::Hash;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::NetClient;

/// Download operation handle
pub struct Download {
    url: Url,
}

/// Result of a download operation
#[derive(Debug)]
pub struct DownloadResult {
    pub url: String,
    pub size: u64,
    pub hash: Hash,
}

impl Download {
    /// Create a new download
    ///
    /// # Errors
    ///
    /// Returns an error if the provided URL is invalid or cannot be parsed.
    pub fn new(url: &str) -> Result<Self, Error> {
        let url = Url::parse(url).map_err(|e| NetworkError::InvalidUrl(e.to_string()))?;
        Ok(Self { url })
    }

    /// Execute the download
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the server returns an error status,
    /// the file cannot be created or written to, the hash verification fails (if expected),
    /// or there are I/O errors during the download process.
    pub async fn execute(
        self,
        client: &NetClient,
        dest: &Path,
        expected_hash: Option<&Hash>,
        tx: &EventSender,
    ) -> Result<DownloadResult, Error> {
        let url_str = self.url.to_string();

        // Start download
        let response = client.get(url_str.as_str()).await?;

        if !response.status().is_success() {
            return Err(NetworkError::HttpError {
                status: response.status().as_u16(),
                message: response.status().to_string(),
            }
            .into());
        }

        // Get content length if available
        let content_length = response.content_length();

        tx.emit(Event::DownloadStarted {
            url: url_str.clone(),
            size: content_length,
        });

        // Create parent directory if needed
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Create temporary file
        let temp_path = dest.with_extension("download");
        let mut file = File::create(&temp_path).await?;

        // Download with progress
        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;
        let mut hasher = blake3::Hasher::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| NetworkError::DownloadFailed(e.to_string()))?;

            // Update hash
            hasher.update(&chunk);

            // Write to file
            file.write_all(&chunk).await?;

            // Update progress
            downloaded += chunk.len() as u64;

            if let Some(total) = content_length {
                tx.emit(Event::DownloadProgress {
                    url: url_str.clone(),
                    bytes_downloaded: downloaded,
                    total_bytes: total,
                });
            }
        }

        // Ensure all data is written
        file.flush().await?;
        drop(file);

        // Calculate final hash
        let hash = Hash::from_bytes(*hasher.finalize().as_bytes());

        // Verify hash if expected
        if let Some(expected) = expected_hash {
            if hash != *expected {
                // Clean up temp file
                let _ = tokio::fs::remove_file(&temp_path).await;

                return Err(NetworkError::ChecksumMismatch {
                    expected: expected.to_hex(),
                    actual: hash.to_hex(),
                }
                .into());
            }
        }

        // Move to final destination
        tokio::fs::rename(&temp_path, dest).await?;

        tx.emit(Event::DownloadCompleted {
            url: url_str.clone(),
            size: downloaded,
        });

        Ok(DownloadResult {
            url: url_str,
            size: downloaded,
            hash,
        })
    }
}

/// Download multiple files concurrently
///
/// This function is kept for future use when parallel package downloads are implemented
#[allow(dead_code)]
pub async fn download_batch(
    client: &NetClient,
    downloads: Vec<(String, String, Option<Hash>)>, // (url, path, hash)
    max_concurrent: usize,
    tx: EventSender,
) -> Result<Vec<DownloadResult>, Error> {
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();
    let mut results = Vec::with_capacity(downloads.len());

    for (url, path, hash) in downloads {
        let client = client.clone();
        let tx = tx.clone();
        let path = Path::new(&path).to_path_buf();

        let fut = async move {
            let download = Download::new(&url)?;
            download.execute(&client, &path, hash.as_ref(), &tx).await
        };

        futures.push(fut);

        // Limit concurrency
        if futures.len() >= max_concurrent {
            if let Some(result) = futures.next().await {
                results.push(result?);
            }
        }
    }

    // Collect remaining results
    while let Some(result) = futures.next().await {
        results.push(result?);
    }

    Ok(results)
}
