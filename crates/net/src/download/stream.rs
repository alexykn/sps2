//! Low-level streaming download mechanics

use super::config::{DownloadResult, StreamParams};
use super::resume::calculate_existing_file_hash;
use futures::StreamExt;
use sps2_errors::{Error, NetworkError};
use sps2_events::{AppEvent, DownloadEvent, EventEmitter};
use sps2_hash::Hash;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::{self as tokio_fs, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, SeekFrom};

/// Stream download with progress reporting and hash calculation
pub(super) async fn stream_download(
    config: &super::config::PackageDownloadConfig,
    response: reqwest::Response,
    dest_path: &Path,
    resume_offset: u64,
    params: &StreamParams<'_>,
) -> Result<DownloadResult, Error> {
    // Open file for writing (append if resuming)
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

    // Initialize progress tracking
    let downloaded = Arc::new(AtomicU64::new(resume_offset));
    let _start_time = Instant::now();
    let mut last_progress_update = Instant::now();
    let mut first_chunk = true;

    // Initialize hash calculation
    let mut hasher = blake3::Hasher::new();

    // If resuming, we need to rehash the existing file content
    if resume_offset > 0 {
        let existing_hash = calculate_existing_file_hash(config, dest_path, resume_offset).await?;
        hasher = existing_hash;
    }

    // Stream the response
    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| NetworkError::DownloadFailed(e.to_string()))?;

        // Update hash
        hasher.update(&chunk);

        // Write to file
        file.write_all(&chunk).await?;

        // Update progress
        let current_downloaded =
            downloaded.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;

        // Emit progress events (throttled to avoid spam, but always emit first chunk)
        if first_chunk || last_progress_update.elapsed() >= Duration::from_millis(50) {
            if let (Some(progress_id), Some(progress_manager)) =
                (params.progress_id.as_ref(), params.progress_manager)
            {
                progress_manager.update(progress_id, current_downloaded);
            }
            params
                .event_sender
                .emit(AppEvent::Download(DownloadEvent::Progress {
                    url: params.url.to_string(),
                    bytes_downloaded: current_downloaded,
                    total_bytes: params.total_size,
                    current_speed: 0.0, // TODO: Calculate actual speed
                    average_speed: 0.0, // TODO: Calculate actual speed
                    eta: None,          // TODO: Calculate ETA
                }));
            last_progress_update = Instant::now();
            first_chunk = false;
        }
    }

    // Ensure all data is written
    file.flush().await?;
    drop(file);

    let final_downloaded = downloaded.load(Ordering::Relaxed);

    // Emit final progress event to ensure 100% completion is reported
    params
        .event_sender
        .emit(AppEvent::Download(DownloadEvent::Progress {
            url: params.url.to_string(),
            bytes_downloaded: final_downloaded,
            total_bytes: params.total_size,
            current_speed: 0.0, // TODO: Calculate actual speed
            average_speed: 0.0, // TODO: Calculate actual speed
            eta: None,          // TODO: Calculate ETA
        }));

    let final_hash = Hash::from_blake3_bytes(*hasher.finalize().as_bytes());

    // Verify hash if expected
    if let Some(expected) = params.expected_hash {
        if final_hash != *expected {
            // Clean up file on hash mismatch
            let _ = tokio_fs::remove_file(dest_path).await;
            return Err(NetworkError::ChecksumMismatch {
                expected: expected.to_hex(),
                actual: final_hash.to_hex(),
            }
            .into());
        }
    }

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
