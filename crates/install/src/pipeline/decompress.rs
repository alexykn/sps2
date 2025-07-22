//! Decompression and validation pipeline stage

use crate::pipeline::download::DownloadResult;
use crate::{validate_sp_file, ValidationResult};
use async_compression::tokio::bufread::ZstdDecoder;
use sps2_errors::{Error, InstallError};
use sps2_events::{AppEvent, EventEmitter, EventSender, GeneralEvent};
use sps2_hash::Hash;
use sps2_resolver::PackageId;
use sps2_resources::ResourceManager;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

/// Result of package decompression operation
#[derive(Debug)]
pub struct DecompressResult {
    pub package_id: PackageId,
    pub decompressed_path: PathBuf,
    pub validation_result: ValidationResult,
    pub hash: Hash,
    #[allow(dead_code)] // Held for automatic cleanup on drop
    pub temp_file: Option<tempfile::NamedTempFile>,
}

/// Decompression pipeline stage coordinator
pub struct DecompressPipeline {
    resources: Arc<ResourceManager>,
    buffer_size: usize,
    enable_streaming: bool,
}

impl DecompressPipeline {
    /// Create a new decompress pipeline
    pub fn new(
        resources: Arc<ResourceManager>,
        buffer_size: usize,
        enable_streaming: bool,
    ) -> Self {
        Self {
            resources,
            buffer_size,
            enable_streaming,
        }
    }

    /// Execute streaming decompress and validation pipeline
    pub async fn execute_streaming_decompress_validate(
        &self,
        download_results: &[DownloadResult],
        progress_id: &str,
        tx: &EventSender,
    ) -> Result<Vec<DecompressResult>, Error> {
        if self.enable_streaming {
            self.execute_streaming_mode(download_results, progress_id, tx)
                .await
        } else {
            self.execute_sequential_mode(download_results, progress_id, tx)
                .await
        }
    }

    /// Execute in streaming mode with concurrent decompression
    async fn execute_streaming_mode(
        &self,
        download_results: &[DownloadResult],
        _progress_id: &str,
        tx: &EventSender,
    ) -> Result<Vec<DecompressResult>, Error> {
        let mut results = Vec::new();
        let mut handles = Vec::new();

        for download_result in download_results {
            // Need to move ownership of the download result
            let result_moved = DownloadResult {
                package_id: download_result.package_id.clone(),
                downloaded_path: download_result.downloaded_path.clone(),
                hash: download_result.hash.clone(),
                temp_dir: None, // Can't clone TempDir, so we don't pass it
                node: download_result.node.clone(),
            };

            let handle = self.spawn_streaming_decompress_task(result_moved, tx.clone());
            handles.push(handle);
        }

        // Collect results
        for handle in handles {
            let result = handle.await.map_err(|e| InstallError::TaskError {
                message: format!("decompress task failed: {e}"),
            })??;
            results.push(result);
        }

        Ok(results)
    }

    /// Execute in sequential mode as fallback
    async fn execute_sequential_mode(
        &self,
        download_results: &[DownloadResult],
        _progress_id: &str,
        tx: &EventSender,
    ) -> Result<Vec<DecompressResult>, Error> {
        let mut results = Vec::new();

        for download_result in download_results {
            // Basic validation without streaming
            let validation_result =
                validate_sp_file(&download_result.downloaded_path, Some(tx)).await?;

            if !validation_result.is_valid {
                return Err(InstallError::InvalidPackageFile {
                    path: download_result.downloaded_path.display().to_string(),
                    message: "validation failed".to_string(),
                }
                .into());
            }

            results.push(DecompressResult {
                package_id: download_result.package_id.clone(),
                decompressed_path: download_result.downloaded_path.clone(),
                validation_result,
                hash: download_result.hash.clone(),
                temp_file: None,
            });

            // Update progress would go here if we had the progress manager reference
            // self.progress_manager.update_progress(
            //     progress_id,
            //     results.len() as u64,
            //     Some(download_results.len() as u64),
            //     tx,
            // );
        }

        Ok(results)
    }

    /// Spawn streaming decompress task
    fn spawn_streaming_decompress_task(
        &self,
        download_result: DownloadResult,
        tx: EventSender,
    ) -> JoinHandle<Result<DecompressResult, Error>> {
        let resources = self.resources.clone();
        let buffer_size = self.buffer_size;

        tokio::spawn(async move {
            let _decompress_permit = resources.acquire_decompression_permit().await?;

            // Track memory usage for decompression
            let decompress_memory = buffer_size as u64 * 4; // Estimate 4x buffer for decompression
            resources
                .memory_usage
                .fetch_add(decompress_memory, Ordering::Relaxed);

            // Create streaming decompression pipeline
            let result = Self::streaming_decompress_validate(
                &download_result,
                buffer_size,
                &resources.installation_semaphore,
                &tx,
            )
            .await;

            resources
                .memory_usage
                .fetch_sub(decompress_memory, Ordering::Relaxed);

            result
        })
    }

    /// Perform streaming decompression with concurrent validation
    async fn streaming_decompress_validate(
        download_result: &DownloadResult,
        buffer_size: usize,
        validation_semaphore: &Semaphore,
        tx: &EventSender,
    ) -> Result<DecompressResult, Error> {
        // Create temporary file for decompressed content
        let temp_file =
            tempfile::NamedTempFile::new().map_err(|e| InstallError::TempFileError {
                message: format!("failed to create temp file for decompression: {e}"),
            })?;

        let temp_path = temp_file.path().to_path_buf();

        // Open input file
        let input_file = File::open(&download_result.downloaded_path)
            .await
            .map_err(|e| InstallError::InvalidPackageFile {
                path: download_result.downloaded_path.display().to_string(),
                message: format!("failed to open downloaded file: {e}"),
            })?;

        // Create output file
        let mut output_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)
            .await
            .map_err(|e| InstallError::TempFileError {
                message: format!("failed to create output file: {e}"),
            })?;

        // Set up streaming decompression
        let reader = BufReader::with_capacity(buffer_size, input_file);
        let mut decoder = ZstdDecoder::new(reader);
        let mut buffer = vec![0u8; buffer_size];

        tx.emit(AppEvent::General(GeneralEvent::OperationStarted {
            operation: format!("Streaming decompress: {}", download_result.package_id.name),
        }));

        // Stream decompress with progress
        loop {
            let bytes_read =
                decoder
                    .read(&mut buffer)
                    .await
                    .map_err(|e| InstallError::InvalidPackageFile {
                        path: download_result.downloaded_path.display().to_string(),
                        message: format!("decompression failed: {e}"),
                    })?;

            if bytes_read == 0 {
                break; // End of stream
            }

            output_file
                .write_all(&buffer[..bytes_read])
                .await
                .map_err(|e| InstallError::TempFileError {
                    message: format!("failed to write decompressed data: {e}"),
                })?;
        }

        output_file
            .flush()
            .await
            .map_err(|e| InstallError::TempFileError {
                message: format!("failed to flush decompressed data: {e}"),
            })?;

        drop(output_file); // Close file for validation

        tx.emit(AppEvent::General(GeneralEvent::OperationCompleted {
            operation: format!(
                "Streaming decompress completed: {}",
                download_result.package_id.name
            ),
            success: true,
        }));

        // Concurrent validation
        let _validation_permit =
            validation_semaphore
                .acquire()
                .await
                .map_err(|_| InstallError::ConcurrencyError {
                    message: "failed to acquire validation semaphore".to_string(),
                })?;

        tx.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: About to validate decompressed tar file: {}",
                temp_path.display()
            ),
            context: std::collections::HashMap::new(),
        }));

        // Validate the decompressed tar content, not as a .sp file
        let mut validation_result = crate::ValidationResult::new(crate::PackageFormat::PlainTar);
        crate::validate_tar_archive_content(&temp_path, &mut validation_result).await?;
        validation_result.mark_valid();

        tx.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Validation result: valid={}, file_count={}, size={}",
                validation_result.is_valid,
                validation_result.file_count,
                validation_result.extracted_size
            ),
            context: std::collections::HashMap::new(),
        }));

        if !validation_result.is_valid {
            return Err(InstallError::InvalidPackageFile {
                path: temp_path.display().to_string(),
                message: "validation failed after decompression".to_string(),
            }
            .into());
        }

        Ok(DecompressResult {
            package_id: download_result.package_id.clone(),
            decompressed_path: temp_path,
            validation_result,
            hash: download_result.hash.clone(),
            temp_file: Some(temp_file),
        })
    }
}
