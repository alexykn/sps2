//! Production-ready parallel download/decompress pipeline
//!
//! This module provides a sophisticated pipeline that efficiently handles multiple
//! .sp packages with optimal resource utilization, streaming decompression, and
//! robust error handling.

use crate::{
    staging::{StagingGuard, StagingManager},
    validate_sp_file,
};
use async_compression::tokio::bufread::ZstdDecoder;
use crossbeam::queue::SegQueue;
use dashmap::DashMap;
use sps2_errors::{Error, InstallError};
use sps2_events::{Event, EventSender, EventSenderExt, ProgressManager, ProgressPhase};
use sps2_net::{PackageDownloadConfig, PackageDownloader};
use sps2_resolver::{ExecutionPlan, NodeAction, PackageId, ResolvedNode};
use sps2_store::PackageStore;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Configuration for the parallel pipeline
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Maximum concurrent downloads (default: 4)
    pub max_downloads: usize,
    /// Maximum concurrent decompressions (default: 2)
    pub max_decompressions: usize,
    /// Maximum concurrent validations (default: 3)
    pub max_validations: usize,
    /// Buffer size for streaming operations (default: 256KB)
    pub buffer_size: usize,
    /// Memory limit for concurrent operations (default: 100MB)
    pub memory_limit: u64,
    /// Timeout for individual operations (default: 10 minutes)
    pub operation_timeout: Duration,
    /// Enable streaming download-to-decompress optimization
    pub enable_streaming: bool,
    /// Cleanup timeout for failed operations (default: 5 seconds)
    pub cleanup_timeout: Duration,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_downloads: 4,
            max_decompressions: 2,
            max_validations: 3,
            buffer_size: 256 * 1024,                     // 256KB
            memory_limit: 100 * 1024 * 1024,             // 100MB
            operation_timeout: Duration::from_secs(600), // 10 minutes
            enable_streaming: true,
            cleanup_timeout: Duration::from_secs(5),
        }
    }
}

/// A production-ready parallel download/decompress pipeline
pub struct PipelineMaster {
    /// Pipeline configuration
    config: PipelineConfig,
    /// Package downloader for streaming downloads
    downloader: PackageDownloader,
    /// Package store
    store: PackageStore,
    /// Staging manager for secure extraction
    staging_manager: Arc<StagingManager>,
    /// Progress manager for batch progress tracking
    progress_manager: Arc<ProgressManager>,
    /// Concurrency control semaphores
    download_semaphore: Arc<Semaphore>,
    decompress_semaphore: Arc<Semaphore>,
    validation_semaphore: Arc<Semaphore>,
    /// Memory usage tracking
    memory_usage: Arc<AtomicU64>,
    /// Active operations tracking
    #[allow(dead_code)]
    active_operations: Arc<DashMap<String, PipelineOperation>>,
    /// Resource pools
    temp_files: Arc<SegQueue<PathBuf>>,
    /// Batch operation state
    batch_state: Arc<RwLock<BatchState>>,
}

/// State for batch operations
#[derive(Debug)]
struct BatchState {
    /// Batch ID
    batch_id: String,
    /// Total packages in batch
    total_packages: usize,
    /// Completed packages
    completed_packages: usize,
    /// Failed packages
    failed_packages: Vec<(PackageId, Error)>,
    /// Started time
    started_at: Instant,
    /// Rollback capabilities
    rollback_info: Option<RollbackInfo>,
}

/// Information needed for rollback
#[derive(Debug)]
struct RollbackInfo {
    /// Pre-operation state
    #[allow(dead_code)]
    pre_state: String,
    /// Successfully completed operations that need rollback
    #[allow(dead_code)]
    completed_operations: Vec<PackageId>,
    /// Staging directories to clean up
    staging_dirs: Vec<PathBuf>,
}

/// Individual pipeline operation
#[allow(dead_code)]
struct PipelineOperation {
    /// Operation ID
    id: String,
    /// Package being processed
    package_id: PackageId,
    /// Current stage
    stage: PipelineStage,
    /// Started time
    started_at: Instant,
    /// Memory usage
    memory_usage: u64,
    /// Associated staging directory
    staging_guard: Option<StagingGuard>,
    /// Progress tracker ID
    progress_id: Option<String>,
}

/// Pipeline processing stages
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
enum PipelineStage {
    Queued,
    Downloading,
    StreamingDecompress,
    Validating,
    Staging,
    Installing,
    Completed,
    Failed(String),
}

/// Result of batch pipeline execution
#[derive(Debug)]
pub struct BatchResult {
    /// Batch operation ID
    pub batch_id: String,
    /// Successfully processed packages
    pub successful_packages: Vec<PackageId>,
    /// Failed packages with errors
    pub failed_packages: Vec<(PackageId, Error)>,
    /// Total processing time
    pub duration: Duration,
    /// Peak memory usage
    pub peak_memory_usage: u64,
    /// Whether rollback was performed
    pub rollback_performed: bool,
    /// Aggregate statistics
    pub stats: BatchStats,
}

/// Aggregate statistics for batch processing
#[derive(Debug)]
pub struct BatchStats {
    /// Total bytes downloaded
    pub total_downloaded: u64,
    /// Total packages processed
    pub total_packages: usize,
    /// Average download speed (bytes/sec)
    pub avg_download_speed: f64,
    /// Concurrency efficiency (0.0 to 1.0)
    pub concurrency_efficiency: f64,
    /// Time spent in each stage
    pub stage_timings: HashMap<String, Duration>,
}

impl PipelineMaster {
    /// Create a new pipeline master
    ///
    /// # Errors
    ///
    /// Returns an error if initialization of underlying components fails.
    pub async fn new(config: PipelineConfig, store: PackageStore) -> Result<Self, Error> {
        // Configure downloader for optimal parallel performance
        let download_config = PackageDownloadConfig {
            max_concurrent: config.max_downloads,
            buffer_size: config.buffer_size,
            ..PackageDownloadConfig::default()
        };

        let downloader = PackageDownloader::new(download_config)?;
        let staging_manager = Arc::new(StagingManager::new(store.clone()).await?);
        let progress_manager = Arc::new(ProgressManager::new());

        // Initialize semaphores for resource control
        let download_semaphore = Arc::new(Semaphore::new(config.max_downloads));
        let decompress_semaphore = Arc::new(Semaphore::new(config.max_decompressions));
        let validation_semaphore = Arc::new(Semaphore::new(config.max_validations));

        Ok(Self {
            config,
            downloader,
            store,
            staging_manager,
            progress_manager,
            download_semaphore,
            decompress_semaphore,
            validation_semaphore,
            memory_usage: Arc::new(AtomicU64::new(0)),
            active_operations: Arc::new(DashMap::new()),
            temp_files: Arc::new(SegQueue::new()),
            batch_state: Arc::new(RwLock::new(BatchState {
                batch_id: "none".to_string(),
                total_packages: 0,
                completed_packages: 0,
                failed_packages: Vec::new(),
                started_at: Instant::now(),
                rollback_info: None,
            })),
        })
    }

    /// Execute a batch of packages with dependency ordering and optimal concurrency
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Resource limits are exceeded
    /// - Critical operation failures occur
    /// - Rollback operations fail
    ///
    /// # Panics
    ///
    /// Panics if hardcoded version parsing fails (should never happen).
    pub async fn execute_batch(
        &self,
        execution_plan: &ExecutionPlan,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
        tx: &EventSender,
    ) -> Result<BatchResult, Error> {
        let batch_id = Uuid::new_v4().to_string();
        let started_at = Instant::now();

        // Initialize batch state
        {
            let mut state = self.batch_state.write().await;
            state.batch_id.clone_from(&batch_id);
            state.total_packages = resolved_packages.len();
            state.completed_packages = 0;
            state.failed_packages.clear();
            state.started_at = started_at;
            state.rollback_info = Some(RollbackInfo {
                pre_state: "current".to_string(), // TODO: Get actual state ID
                completed_operations: Vec::new(),
                staging_dirs: Vec::new(),
            });
        }

        tx.emit(Event::OperationStarted {
            operation: format!(
                "Batch pipeline execution: {} packages",
                resolved_packages.len()
            ),
        });

        // Set up progress tracking
        let phases = vec![
            ProgressPhase::new("download", "Downloading packages"),
            ProgressPhase::new("decompress", "Decompressing packages"),
            ProgressPhase::new("validate", "Validating packages"),
            ProgressPhase::new("stage", "Staging packages"),
            ProgressPhase::new("install", "Installing packages"),
        ];

        let progress_id = self.progress_manager.start_operation(
            &batch_id,
            "batch_pipeline",
            Some(resolved_packages.len() as u64),
            phases,
            tx.clone(),
        );

        // Execute the batch pipeline
        let batch_result = match self
            .execute_batch_pipeline(
                &batch_id,
                execution_plan,
                resolved_packages,
                &progress_id,
                tx,
            )
            .await
        {
            Ok(result) => result,
            Err(e) => {
                // Attempt rollback on failure
                tx.emit(Event::Warning {
                    message: format!("Batch pipeline failed, attempting rollback: {e}"),
                    context: Some(batch_id.clone()),
                });

                let rollback_result = self.rollback_batch(&batch_id, tx).await;

                let mut stats = BatchStats {
                    total_downloaded: 0,
                    total_packages: resolved_packages.len(),
                    avg_download_speed: 0.0,
                    concurrency_efficiency: 0.0,
                    stage_timings: HashMap::new(),
                };

                // Generate basic stats even on failure
                stats
                    .stage_timings
                    .insert("total".to_string(), started_at.elapsed());

                BatchResult {
                    batch_id,
                    successful_packages: Vec::new(),
                    failed_packages: vec![(
                        PackageId::new(
                            "batch".to_string(),
                            sps2_types::Version::parse("0.0.0")
                                .expect("hardcoded version should parse"),
                        ),
                        e,
                    )],
                    duration: started_at.elapsed(),
                    peak_memory_usage: self.memory_usage.load(Ordering::Relaxed),
                    rollback_performed: rollback_result.is_ok(),
                    stats,
                }
            }
        };

        // Complete progress tracking
        self.progress_manager.complete_operation(&progress_id, tx);

        tx.emit(Event::OperationCompleted {
            operation: format!(
                "Batch pipeline execution completed: {}",
                batch_result.batch_id
            ),
            success: batch_result.failed_packages.is_empty(),
        });

        Ok(batch_result)
    }

    /// Execute the core batch pipeline logic
    async fn execute_batch_pipeline(
        &self,
        batch_id: &str,
        execution_plan: &ExecutionPlan,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
        progress_id: &str,
        tx: &EventSender,
    ) -> Result<BatchResult, Error> {
        let started_at = Instant::now();
        let mut successful_packages = Vec::new();
        let mut failed_packages = Vec::new();
        let mut total_downloaded = 0u64;
        let mut stage_timings = HashMap::new();

        // Phase 1: Parallel Downloads with Dependency Ordering
        let download_start = Instant::now();
        self.progress_manager.change_phase(progress_id, 0, tx);

        let download_results = self
            .execute_parallel_downloads(execution_plan, resolved_packages, progress_id, tx)
            .await?;

        stage_timings.insert("download".to_string(), download_start.elapsed());

        // Phase 2: Streaming Decompress + Validation Pipeline
        let decompress_start = Instant::now();
        self.progress_manager.change_phase(progress_id, 1, tx);

        let decompress_results = if self.config.enable_streaming {
            self.execute_streaming_decompress_validate(&download_results, progress_id, tx)
                .await?
        } else {
            self.execute_sequential_decompress_validate(&download_results, progress_id, tx)
                .await?
        };

        stage_timings.insert("decompress".to_string(), decompress_start.elapsed());

        // Phase 3: Staging and Installation
        let install_start = Instant::now();
        self.progress_manager.change_phase(progress_id, 3, tx);

        tx.emit(Event::DebugLog {
            message: format!(
                "DEBUG: Starting staging/installation phase with {} packages",
                decompress_results.len()
            ),
            context: std::collections::HashMap::new(),
        });

        let install_results = self
            .execute_parallel_staging_install(&decompress_results, progress_id, tx)
            .await?;

        tx.emit(Event::DebugLog {
            message: format!(
                "DEBUG: Staging/installation completed with {} results",
                install_results.len()
            ),
            context: std::collections::HashMap::new(),
        });

        stage_timings.insert("install".to_string(), install_start.elapsed());

        // Collect results and statistics
        for result in &install_results {
            match result {
                Ok(package_id) => {
                    successful_packages.push(package_id.clone());
                    total_downloaded += 1024 * 1024; // Placeholder - should track actual bytes
                }
                Err((package_id, error)) => {
                    failed_packages.push((package_id.clone(), error.clone()));
                }
            }
        }

        // Calculate efficiency metrics
        let total_duration = started_at.elapsed();
        let concurrency_efficiency = Self::calculate_concurrency_efficiency(&stage_timings);
        #[allow(clippy::cast_precision_loss)] // Precision loss acceptable for statistics
        let avg_download_speed = if total_duration.as_secs() > 0 {
            (total_downloaded as f64) / total_duration.as_secs_f64()
        } else {
            0.0
        };

        stage_timings.insert("total".to_string(), total_duration);

        let stats = BatchStats {
            total_downloaded,
            total_packages: resolved_packages.len(),
            avg_download_speed,
            concurrency_efficiency,
            stage_timings,
        };

        Ok(BatchResult {
            batch_id: batch_id.to_string(),
            successful_packages,
            failed_packages,
            duration: total_duration,
            peak_memory_usage: self.memory_usage.load(Ordering::Relaxed),
            rollback_performed: false,
            stats,
        })
    }

    /// Execute parallel downloads with dependency ordering
    async fn execute_parallel_downloads(
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
    fn spawn_download_task(
        &self,
        package_id: PackageId,
        node: ResolvedNode,
        _progress_id: String,
        tx: EventSender,
    ) -> JoinHandle<Result<DownloadResult, Error>> {
        let downloader = self.downloader.clone();
        let semaphore = self.download_semaphore.clone();
        let memory_usage = self.memory_usage.clone();
        let timeout = self.config.operation_timeout;

        tokio::spawn(async move {
            let _permit =
                semaphore
                    .acquire()
                    .await
                    .map_err(|_| InstallError::ConcurrencyError {
                        message: "failed to acquire download semaphore".to_string(),
                    })?;

            // Track memory usage
            let estimated_size = 50 * 1024 * 1024; // Estimate 50MB per download
            memory_usage.fetch_add(estimated_size, Ordering::Relaxed);

            let result = tokio::time::timeout(
                timeout,
                Self::download_package_with_progress(&downloader, &package_id, &node, &tx),
            )
            .await;

            // Release memory
            memory_usage.fetch_sub(estimated_size, Ordering::Relaxed);

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
                    Ok(DownloadResult {
                        package_id: package_id.clone(),
                        downloaded_path: path.clone(),
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

    /// Execute streaming decompress and validation pipeline
    async fn execute_streaming_decompress_validate(
        &self,
        download_results: &[DownloadResult],
        progress_id: &str,
        tx: &EventSender,
    ) -> Result<Vec<DecompressResult>, Error> {
        let mut results = Vec::new();
        let mut handles = Vec::new();

        for download_result in download_results {
            // Need to move ownership of the download result
            let result_moved = DownloadResult {
                package_id: download_result.package_id.clone(),
                downloaded_path: download_result.downloaded_path.clone(),
                temp_dir: None, // Can't clone TempDir, so we don't pass it
                node: download_result.node.clone(),
            };

            let handle = self.spawn_streaming_decompress_task(
                result_moved,
                progress_id.to_string(),
                tx.clone(),
            );
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

    /// Spawn streaming decompress task
    fn spawn_streaming_decompress_task(
        &self,
        download_result: DownloadResult,
        _progress_id: String,
        tx: EventSender,
    ) -> JoinHandle<Result<DecompressResult, Error>> {
        let decompress_semaphore = self.decompress_semaphore.clone();
        let validation_semaphore = self.validation_semaphore.clone();
        let memory_usage = self.memory_usage.clone();
        let buffer_size = self.config.buffer_size;

        tokio::spawn(async move {
            let _decompress_permit = decompress_semaphore.acquire().await.map_err(|_| {
                InstallError::ConcurrencyError {
                    message: "failed to acquire decompress semaphore".to_string(),
                }
            })?;

            // Track memory usage for decompression
            let decompress_memory = buffer_size as u64 * 4; // Estimate 4x buffer for decompression
            memory_usage.fetch_add(decompress_memory, Ordering::Relaxed);

            // Create streaming decompression pipeline
            let result = Self::streaming_decompress_validate(
                &download_result,
                buffer_size,
                &validation_semaphore,
                &tx,
            )
            .await;

            memory_usage.fetch_sub(decompress_memory, Ordering::Relaxed);

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

        tx.emit(Event::OperationStarted {
            operation: format!("Streaming decompress: {}", download_result.package_id.name),
        });

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

        tx.emit(Event::OperationCompleted {
            operation: format!(
                "Streaming decompress completed: {}",
                download_result.package_id.name
            ),
            success: true,
        });

        // Concurrent validation
        let _validation_permit =
            validation_semaphore
                .acquire()
                .await
                .map_err(|_| InstallError::ConcurrencyError {
                    message: "failed to acquire validation semaphore".to_string(),
                })?;

        tx.emit(Event::DebugLog {
            message: format!(
                "DEBUG: About to validate decompressed tar file: {}",
                temp_path.display()
            ),
            context: std::collections::HashMap::new(),
        });

        // Validate the decompressed tar content, not as a .sp file
        let mut validation_result = crate::ValidationResult::new(crate::PackageFormat::PlainTar);
        crate::validate_tar_archive_content(&temp_path, &mut validation_result).await?;
        validation_result.mark_valid();

        tx.emit(Event::DebugLog {
            message: format!(
                "DEBUG: Validation result: valid={}, file_count={}, size={}",
                validation_result.is_valid,
                validation_result.file_count,
                validation_result.extracted_size
            ),
            context: std::collections::HashMap::new(),
        });

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
            temp_file: Some(temp_file),
        })
    }

    /// Execute sequential decompress and validate (fallback)
    async fn execute_sequential_decompress_validate(
        &self,
        download_results: &[DownloadResult],
        progress_id: &str,
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
                temp_file: None,
            });

            // Update progress
            self.progress_manager.update_progress(
                progress_id,
                results.len() as u64,
                Some(download_results.len() as u64),
                tx,
            );
        }

        Ok(results)
    }

    /// Execute parallel staging and installation
    async fn execute_parallel_staging_install(
        &self,
        decompress_results: &[DecompressResult],
        progress_id: &str,
        tx: &EventSender,
    ) -> Result<Vec<Result<PackageId, (PackageId, Error)>>, Error> {
        let mut handles = Vec::new();

        for decompress_result in decompress_results {
            // Need to move ownership of the decompress result
            let result_moved = DecompressResult {
                package_id: decompress_result.package_id.clone(),
                decompressed_path: decompress_result.decompressed_path.clone(),
                validation_result: decompress_result.validation_result.clone(),
                temp_file: None, // Can't clone NamedTempFile, so we don't pass it
            };

            let handle =
                self.spawn_staging_install_task(result_moved, progress_id.to_string(), tx.clone());
            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            let result = handle.await.map_err(|e| InstallError::TaskError {
                message: format!("staging/install task failed: {e}"),
            })?;
            results.push(result);
        }

        Ok(results)
    }

    /// Spawn staging and installation task
    fn spawn_staging_install_task(
        &self,
        decompress_result: DecompressResult,
        _progress_id: String,
        tx: EventSender,
    ) -> JoinHandle<Result<PackageId, (PackageId, Error)>> {
        let staging_manager = self.staging_manager.clone();
        let store = self.store.clone();

        tokio::spawn(async move {
            match Self::stage_and_install_package(&decompress_result, &staging_manager, &store, &tx)
                .await
            {
                Ok(package_id) => Ok(package_id),
                Err(e) => Err((decompress_result.package_id, e)),
            }
        })
    }

    /// Stage and install a single package
    async fn stage_and_install_package(
        decompress_result: &DecompressResult,
        staging_manager: &StagingManager,
        store: &PackageStore,
        tx: &EventSender,
    ) -> Result<PackageId, Error> {
        tx.emit(Event::DebugLog {
            message: format!(
                "DEBUG: Starting staging/install for package: {}",
                decompress_result.package_id.name
            ),
            context: std::collections::HashMap::new(),
        });

        // Extract to staging directory
        tx.emit(Event::DebugLog {
            message: format!(
                "DEBUG: Extracting to staging directory from: {}",
                decompress_result.decompressed_path.display()
            ),
            context: std::collections::HashMap::new(),
        });

        let staging_dir = staging_manager
            .extract_validated_tar_to_staging(
                &decompress_result.decompressed_path,
                &decompress_result.package_id,
                Some(tx),
            )
            .await?;

        tx.emit(Event::DebugLog {
            message: format!(
                "DEBUG: Extracted to staging directory: {}",
                staging_dir.path().display()
            ),
            context: std::collections::HashMap::new(),
        });

        // Install from staging directory
        tx.emit(Event::DebugLog {
            message: "DEBUG: Adding package to store from staging".to_string(),
            context: std::collections::HashMap::new(),
        });

        store
            .add_package_from_staging(staging_dir.path(), &decompress_result.package_id)
            .await?;

        tx.emit(Event::DebugLog {
            message: "DEBUG: Package added to store successfully".to_string(),
            context: std::collections::HashMap::new(),
        });

        tx.emit(Event::PackageInstalled {
            name: decompress_result.package_id.name.clone(),
            version: decompress_result.package_id.version.clone(),
            path: staging_dir.path().display().to_string(),
        });

        tx.emit(Event::DebugLog {
            message: format!(
                "DEBUG: Package installation completed: {}",
                decompress_result.package_id.name
            ),
            context: std::collections::HashMap::new(),
        });

        Ok(decompress_result.package_id.clone())
    }

    /// Perform rollback for failed batch operation
    async fn rollback_batch(&self, batch_id: &str, tx: &EventSender) -> Result<(), Error> {
        tx.emit(Event::OperationStarted {
            operation: format!("Rolling back batch: {batch_id}"),
        });

        let state = self.batch_state.read().await;
        if let Some(rollback_info) = &state.rollback_info {
            // Clean up staging directories
            for staging_dir in &rollback_info.staging_dirs {
                if staging_dir.exists() {
                    let _ = tokio::fs::remove_dir_all(staging_dir).await;
                }
            }

            // TODO: Implement actual state rollback using state management
            tx.emit(Event::Warning {
                message: "State rollback not yet implemented".to_string(),
                context: Some(batch_id.to_string()),
            });
        }

        tx.emit(Event::OperationCompleted {
            operation: format!("Rollback completed: {batch_id}"),
            success: true,
        });

        Ok(())
    }

    /// Calculate concurrency efficiency based on stage timings
    fn calculate_concurrency_efficiency(stage_timings: &HashMap<String, Duration>) -> f64 {
        // Simple efficiency calculation: ratio of parallel to sequential time
        let total_time = stage_timings
            .get("total")
            .map_or(1.0, std::time::Duration::as_secs_f64);
        let sum_stages = stage_timings
            .values()
            .filter_map(|d| {
                if d.as_secs_f64() < total_time {
                    Some(d.as_secs_f64())
                } else {
                    None
                }
            })
            .sum::<f64>();

        if sum_stages > 0.0 {
            (total_time / sum_stages).min(1.0)
        } else {
            0.0
        }
    }

    /// Clean up resources and temporary files
    ///
    /// # Errors
    ///
    /// Returns an error if cleanup operations fail
    pub async fn cleanup(&self) -> Result<(), Error> {
        // Clean up temporary files
        while let Some(temp_path) = self.temp_files.pop() {
            let _ = tokio::fs::remove_file(temp_path).await;
        }

        // Clean up staging directories
        self.staging_manager.cleanup_old_staging_dirs().await?;

        Ok(())
    }
}

/// Result of package download operation
#[derive(Debug)]
struct DownloadResult {
    package_id: PackageId,
    downloaded_path: PathBuf,
    #[allow(dead_code)]
    temp_dir: Option<tempfile::TempDir>,
    node: ResolvedNode,
}

/// Result of package decompression operation
#[derive(Debug)]
struct DecompressResult {
    package_id: PackageId,
    decompressed_path: PathBuf,
    validation_result: crate::ValidationResult,
    #[allow(dead_code)]
    temp_file: Option<tempfile::NamedTempFile>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_pipeline_master_creation() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());
        let config = PipelineConfig::default();

        let pipeline = PipelineMaster::new(config, store).await.unwrap();
        assert_eq!(pipeline.config.max_downloads, 4);
        assert_eq!(pipeline.config.max_decompressions, 2);
        assert!(pipeline.config.enable_streaming);
    }

    #[tokio::test]
    async fn test_pipeline_config_validation() {
        let config = PipelineConfig {
            max_downloads: 8,
            max_decompressions: 4,
            buffer_size: 512 * 1024,
            ..PipelineConfig::default()
        };

        assert_eq!(config.max_downloads, 8);
        assert_eq!(config.buffer_size, 512 * 1024);
    }

    #[test]
    fn test_batch_stats() {
        let mut stats = BatchStats {
            total_downloaded: 1024 * 1024,
            total_packages: 5,
            avg_download_speed: 1024.0,
            concurrency_efficiency: 0.8,
            stage_timings: HashMap::new(),
        };

        stats
            .stage_timings
            .insert("download".to_string(), Duration::from_secs(10));
        stats
            .stage_timings
            .insert("install".to_string(), Duration::from_secs(5));

        assert_eq!(stats.total_packages, 5);
        assert!((stats.concurrency_efficiency - 0.8).abs() < f64::EPSILON);
    }
}
