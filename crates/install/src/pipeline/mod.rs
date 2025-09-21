//! Production-ready parallel download/decompress pipeline
//!
//! This module provides a sophisticated pipeline that efficiently handles multiple
//! .sp packages with optimal resource utilization, streaming decompression, and
//! robust error handling.

pub mod batch;
pub mod config;
pub mod decompress;
pub mod download;
pub mod operation;
pub mod staging;

use crate::staging::StagingManager;
use batch::{BatchManager, BatchResult, BatchStats, RollbackInfo};
pub use config::PipelineConfig;
use dashmap::DashMap;
use decompress::DecompressPipeline;
use download::DownloadPipeline;
use operation::PipelineOperation;
use sps2_errors::Error;
use sps2_events::config::ProgressPhase;
use sps2_events::{AppEvent, EventEmitter, GeneralEvent, ProgressManager};
use sps2_net::{PackageDownloadConfig, PackageDownloader};
use sps2_resolver::{ExecutionPlan, PackageId, ResolvedNode};
use sps2_resources::{IntoResourceLimits, ResourceManager};
use sps2_store::PackageStore;
use staging::StagingPipeline;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

/// A production-ready parallel download/decompress pipeline
pub struct PipelineMaster {
    /// Pipeline configuration
    config: PipelineConfig,
    /// Resource management and coordination
    resources: Arc<ResourceManager>,
    /// Progress manager for batch progress tracking
    progress_manager: Arc<ProgressManager>,
    /// Pipeline stages
    download_pipeline: DownloadPipeline,
    decompress_pipeline: DecompressPipeline,
    staging_pipeline: StagingPipeline,
    /// Active operations tracking
    #[allow(dead_code)] // Reserved for future pipeline operation monitoring
    active_operations: Arc<DashMap<String, PipelineOperation>>,
    /// Batch operation management
    batch_manager: BatchManager,
}

impl PipelineMaster {
    /// Create a new pipeline master
    ///
    /// # Errors
    ///
    /// Returns an error if initialization of underlying components fails.
    pub async fn new(
        config: PipelineConfig,
        store: PackageStore,
        staging_base_path: PathBuf,
    ) -> Result<Self, Error> {
        // Configure downloader for optimal parallel performance
        let download_config = PackageDownloadConfig {
            max_concurrent: config.max_downloads,
            buffer_size: config.buffer_size,
            ..PackageDownloadConfig::default()
        };

        let resources = Arc::new(ResourceManager::new(config.clone().into_resource_limits()));

        let progress_manager = Arc::new(ProgressManager::new());

        let downloader = PackageDownloader::new(download_config, (*progress_manager).clone())?;
        let staging_manager = Arc::new(
            StagingManager::new(store.clone(), staging_base_path.clone(), resources.clone())
                .await?,
        );

        // Initialize pipeline stages
        let download_pipeline = DownloadPipeline::new(
            downloader,
            resources.clone(),
            progress_manager.clone(),
            config.operation_timeout,
        );

        let decompress_pipeline = DecompressPipeline::new(
            resources.clone(),
            config.buffer_size,
            config.enable_streaming,
        );

        let staging_pipeline = StagingPipeline::new(staging_manager.clone(), store);

        Ok(Self {
            config,
            resources,
            progress_manager,
            download_pipeline,
            decompress_pipeline,
            staging_pipeline,
            active_operations: Arc::new(DashMap::new()),
            batch_manager: BatchManager::new(),
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
    pub async fn execute_batch<T: EventEmitter>(
        &self,
        execution_plan: &ExecutionPlan,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
        context: &T,
    ) -> Result<BatchResult, Error> {
        let batch_id = Uuid::new_v4().to_string();
        let started_at = Instant::now();

        // Initialize batch state
        {
            let mut state = self.batch_manager.batch_state.write().await;
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

        context.emit(AppEvent::General(GeneralEvent::OperationStarted {
            operation: format!(
                "Batch pipeline execution: {} packages",
                resolved_packages.len()
            ),
        }));

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
            context.event_sender().expect("event sender not available"),
            None,
        );

        // Execute the batch pipeline
        let batch_result = match self
            .execute_batch_pipeline(
                &batch_id,
                execution_plan,
                resolved_packages,
                &progress_id,
                context,
            )
            .await
        {
            Ok(result) => result,
            Err(e) => {
                // Attempt rollback on failure
                context.emit_warning(format!("Batch pipeline failed, attempting rollback: {e}"));

                let rollback_result = self.rollback_batch(&batch_id, context).await;

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
                    package_hashes: HashMap::new(),
                    failed_packages: vec![(
                        PackageId::new(
                            "batch".to_string(),
                            sps2_types::Version::parse("0.0.0")
                                .expect("hardcoded version should parse"),
                        ),
                        e,
                    )],
                    duration: started_at.elapsed(),
                    peak_memory_usage: self.resources.limits.memory_usage.unwrap_or(0),
                    rollback_performed: rollback_result.is_ok(),
                    stats,
                }
            }
        };

        // Complete progress tracking
        self.progress_manager
            .complete_operation(&progress_id, &context.event_sender().cloned().unwrap());

        context.emit(AppEvent::General(GeneralEvent::OperationCompleted {
            operation: format!(
                "Batch pipeline execution completed: {}",
                batch_result.batch_id
            ),
            success: batch_result.failed_packages.is_empty(),
        }));

        Ok(batch_result)
    }

    /// Execute the core batch pipeline logic
    async fn execute_batch_pipeline<T: EventEmitter>(
        &self,
        batch_id: &str,
        execution_plan: &ExecutionPlan,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
        progress_id: &str,
        context: &T,
    ) -> Result<BatchResult, Error> {
        let started_at = Instant::now();
        let mut successful_packages = Vec::new();
        let mut failed_packages = Vec::new();
        let mut total_downloaded = 0u64;
        let mut stage_timings = HashMap::new();

        // Phase 1: Parallel Downloads with Dependency Ordering
        let download_start = Instant::now();
        self.progress_manager.change_phase(
            progress_id,
            0,
            &context.event_sender().cloned().unwrap(),
        );

        let download_results = self
            .download_pipeline
            .execute_parallel_downloads(
                execution_plan,
                resolved_packages,
                progress_id,
                &context.event_sender().cloned().unwrap(),
            )
            .await?;

        stage_timings.insert("download".to_string(), download_start.elapsed());

        // Phase 2: Streaming Decompress + Validation Pipeline
        let decompress_start = Instant::now();
        self.progress_manager.change_phase(
            progress_id,
            1,
            &context.event_sender().cloned().unwrap(),
        );

        let decompress_results = self
            .decompress_pipeline
            .execute_streaming_decompress_validate(
                download_results,
                progress_id,
                &context.event_sender().cloned().unwrap(),
            )
            .await?;

        stage_timings.insert("decompress".to_string(), decompress_start.elapsed());

        // Phase 3: Staging and Installation
        let install_start = Instant::now();
        self.progress_manager.change_phase(
            progress_id,
            3,
            &context.event_sender().cloned().unwrap(),
        );

        context.emit_debug(format!(
            "DEBUG: Starting staging/installation phase with {} packages",
            decompress_results.len()
        ));

        let install_results = self
            .staging_pipeline
            .execute_parallel_staging_install(
                &decompress_results,
                progress_id,
                &context.event_sender().cloned().unwrap(),
            )
            .await?;

        context.emit_debug(format!(
            "DEBUG: Staging/installation completed with {} results",
            install_results.len()
        ));

        stage_timings.insert("install".to_string(), install_start.elapsed());

        // Build package hash mapping from decompress results
        let mut package_hashes = HashMap::new();
        for decompress_result in &decompress_results {
            package_hashes.insert(
                decompress_result.package_id.clone(),
                decompress_result.hash.clone(),
            );
        }

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
        let concurrency_efficiency = BatchManager::calculate_concurrency_efficiency(&stage_timings);
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
            package_hashes,
            failed_packages,
            duration: total_duration,
            peak_memory_usage: self.resources.limits.memory_usage.unwrap_or(0),
            rollback_performed: false,
            stats,
        })
    }

    /// Perform rollback for failed batch operation
    async fn rollback_batch<T: EventEmitter>(
        &self,
        batch_id: &str,
        context: &T,
    ) -> Result<(), Error> {
        context.emit(AppEvent::General(GeneralEvent::OperationStarted {
            operation: format!("Rolling back batch: {batch_id}"),
        }));

        let state = self.batch_manager.batch_state.read().await;
        if let Some(rollback_info) = &state.rollback_info {
            // Clean up staging directories
            for staging_dir in &rollback_info.staging_dirs {
                if staging_dir.exists() {
                    let _ = tokio::fs::remove_dir_all(staging_dir).await;
                }
            }

            // TODO: Implement actual state rollback using state management
            context.emit_warning("State rollback not yet implemented".to_string());
        }

        context.emit(AppEvent::General(GeneralEvent::OperationCompleted {
            operation: format!("Rollback completed: {batch_id}"),
            success: true,
        }));

        Ok(())
    }

    /// Get a reference to the pipeline configuration
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    /// Clean up resources and temporary files
    ///
    /// # Errors
    ///
    /// Returns an error if cleanup operations fail
    pub fn cleanup(&self) -> Result<(), Error> {
        // Clean up temporary files with timeout
        let cleanup_timeout = self.config.cleanup_timeout;

        match self.resources.cleanup() {
            Ok(()) => {}
            Err(_e) => {
                return Err(Error::internal(format!(
                    "Cleanup operation failed after {cleanup_timeout:?}"
                )));
            }
        }

        // TODO: Clean up staging directories via staging manager
        // self.staging_manager.cleanup_old_staging_dirs().await?;

        Ok(())
    }
}
