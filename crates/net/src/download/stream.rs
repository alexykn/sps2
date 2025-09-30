//! Low-level streaming download mechanics

use super::config::{DownloadResult, StreamParams};
use super::resume::calculate_existing_file_hash;
use futures::StreamExt;
use sps2_errors::{Error, NetworkError};

use sps2_hash::Hash;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::{self as tokio_fs, File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, SeekFrom};

/// RAII guard for download lock file - ensures cleanup on drop
struct LockGuard {
    path: std::path::PathBuf,
    _file: File,
}

impl LockGuard {
    async fn new(lock_path: std::path::PathBuf) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true) // Atomic - fails if file already exists
            .open(&lock_path)
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    NetworkError::DownloadFailed(format!(
                        "File {} is already being downloaded by another process",
                        lock_path.display()
                    ))
                } else {
                    NetworkError::DownloadFailed(format!(
                        "Failed to create lock file {}: {e}",
                        lock_path.display()
                    ))
                }
            })?;

        Ok(Self {
            path: lock_path,
            _file: file,
        })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Best-effort cleanup - ignore errors
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Prepare file and hasher for download
async fn prepare_download(
    config: &super::config::PackageDownloadConfig,
    dest_path: &Path,
    resume_offset: u64,
) -> Result<(File, blake3::Hasher), Error> {
    let mut file = if resume_offset > 0 {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(dest_path)
            .await?
    } else {
        tokio_fs::File::create(dest_path).await?
    };

    if resume_offset > 0 {
        file.seek(SeekFrom::End(0)).await?;
    }

    let hasher = if resume_offset > 0 {
        calculate_existing_file_hash(config, dest_path, resume_offset).await?
    } else {
        blake3::Hasher::new()
    };

    Ok((file, hasher))
}

/// Handle progress reporting during download
fn should_report_progress(first_chunk: bool, last_update: &Instant) -> bool {
    first_chunk || last_update.elapsed() >= Duration::from_millis(50)
}

/// Report progress update
fn report_progress(params: &StreamParams<'_>, current_downloaded: u64) {
    if let Some(progress_manager) = params.progress_manager {
        progress_manager.update_progress(
            &params.progress_tracker_id,
            current_downloaded,
            Some(params.total_size),
            params.event_sender,
        );
    }
}

/// Verify download hash matches expected
fn verify_hash(
    final_hash: &Hash,
    expected_hash: Option<&Hash>,
    dest_path: &Path,
) -> Result<(), Error> {
    if let Some(expected) = expected_hash {
        if final_hash != expected {
            let _ = std::fs::remove_file(dest_path);
            return Err(NetworkError::ChecksumMismatch {
                expected: expected.to_hex(),
                actual: final_hash.to_hex(),
            }
            .into());
        }
    }
    Ok(())
}

/// Stream download with progress reporting and hash calculation
pub(super) async fn stream_download(
    config: &super::config::PackageDownloadConfig,
    response: reqwest::Response,
    dest_path: &Path,
    resume_offset: u64,
    params: &StreamParams<'_>,
) -> Result<DownloadResult, Error> {
    // Create lock file atomically to prevent concurrent downloads
    // Lock guard will automatically clean up on drop (including panics/errors)
    let lock_path = dest_path.with_extension("lock");
    let _lock_guard = LockGuard::new(lock_path).await?;

    let (mut file, mut hasher) = prepare_download(config, dest_path, resume_offset).await?;

    // Initialize progress tracking
    let downloaded = Arc::new(AtomicU64::new(resume_offset));
    let mut last_progress_update = Instant::now();
    let mut first_chunk = true;

    // Stream the response
    let mut stream = response.bytes_stream();
    let chunk_timeout = config.chunk_timeout;

    loop {
        let chunk_result = tokio::time::timeout(chunk_timeout, stream.next()).await;

        match chunk_result {
            Ok(Some(chunk_result)) => {
                let chunk =
                    chunk_result.map_err(|e| NetworkError::DownloadFailed(e.to_string()))?;

                hasher.update(&chunk);
                file.write_all(&chunk).await?;

                let current_downloaded = downloaded
                    .fetch_add(chunk.len() as u64, Ordering::Relaxed)
                    + chunk.len() as u64;

                if should_report_progress(first_chunk, &last_progress_update) {
                    report_progress(params, current_downloaded);
                    last_progress_update = Instant::now();
                    first_chunk = false;
                }
            }
            Ok(None) => break,
            Err(_) => {
                return Err(NetworkError::Timeout {
                    url: params.url.to_string(),
                }
                .into());
            }
        }
    }

    file.flush().await?;
    drop(file);

    let final_downloaded = downloaded.load(Ordering::Relaxed);
    report_progress(params, final_downloaded);

    let final_hash = Hash::from_blake3_bytes(*hasher.finalize().as_bytes());
    verify_hash(&final_hash, params.expected_hash, dest_path)?;

    // Lock guard automatically cleaned up on drop
    Ok(DownloadResult {
        hash: final_hash,
        size: final_downloaded,
    })
}

/// Download a simple file (for signatures)
pub(super) async fn download_file_simple(
    client: &crate::client::NetClient,
    url: &str,
    dest_path: &Path,
    _tx: &sps2_events::EventSender,
) -> Result<(), Error> {
    let response = client.get(url).await?;

    if !response.status().is_success() {
        return Err(NetworkError::HttpError {
            status: response.status().as_u16(),
            message: response.status().to_string(),
        }
        .into());
    }

    let content = response
        .bytes()
        .await
        .map_err(|e| NetworkError::DownloadFailed(e.to_string()))?;

    tokio_fs::write(dest_path, content).await?;
    Ok(())
}
