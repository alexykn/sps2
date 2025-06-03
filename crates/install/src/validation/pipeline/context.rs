//! Validation pipeline context management
//!
//! This module manages the context and state for validation pipelines,
//! including configuration, progress tracking, and resource management.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::validation::content::ContentLimits;
use crate::validation::security::SecurityPolicy;
use crate::validation::types::ValidationContext;

/// Pipeline execution context
///
/// This struct maintains the state and configuration for a validation
/// pipeline execution, including timing, progress, and resource tracking.
#[derive(Debug)]
pub struct PipelineContext {
    /// Validation configuration
    pub validation_config: ValidationContext,
    /// Content limits to enforce
    pub content_limits: ContentLimits,
    /// Security policy to apply
    pub security_policy: SecurityPolicy,
    /// Pipeline execution state
    pub execution_state: ExecutionState,
    /// Performance metrics
    pub metrics: PipelineMetrics,
    /// Custom properties
    pub properties: HashMap<String, String>,
}

/// Pipeline execution state
#[derive(Debug, Clone)]
pub struct ExecutionState {
    /// Current stage being executed
    pub current_stage: ValidationStage,
    /// Stages that have been completed
    pub completed_stages: Vec<ValidationStage>,
    /// Whether pipeline is currently running
    pub is_running: bool,
    /// Start time of pipeline execution
    pub start_time: Option<Instant>,
    /// End time of pipeline execution
    pub end_time: Option<Instant>,
    /// Current progress percentage (0-100)
    pub progress_percent: u8,
}

/// Validation pipeline stages
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValidationStage {
    /// Initial setup and preparation
    Initialization,
    /// File format validation
    FormatValidation,
    /// Content validation and inspection
    ContentValidation,
    /// Security validation and policy enforcement
    SecurityValidation,
    /// Final result compilation
    Finalization,
    /// Pipeline completed
    Completed,
}

/// Pipeline performance metrics
#[derive(Debug, Clone, Default)]
pub struct PipelineMetrics {
    /// Total execution time
    pub total_duration: Option<Duration>,
    /// Time spent in each stage
    pub stage_durations: HashMap<ValidationStage, Duration>,
    /// Number of files processed
    pub files_processed: usize,
    /// Bytes processed
    pub bytes_processed: u64,
    /// Number of warnings generated
    pub warning_count: usize,
    /// Number of errors encountered
    pub error_count: usize,
    /// Memory usage peak (bytes)
    pub peak_memory_usage: u64,
}

impl PipelineContext {
    /// Create new pipeline context
    #[must_use]
    pub fn new() -> Self {
        Self {
            validation_config: ValidationContext::default(),
            content_limits: ContentLimits::default(),
            security_policy: SecurityPolicy::default(),
            execution_state: ExecutionState::new(),
            metrics: PipelineMetrics::default(),
            properties: HashMap::new(),
        }
    }

    /// Set validation configuration
    #[must_use]
    pub fn with_validation_config(mut self, config: ValidationContext) -> Self {
        self.validation_config = config;
        self
    }

    /// Set content limits
    #[must_use]
    pub fn with_content_limits(mut self, limits: ContentLimits) -> Self {
        self.content_limits = limits;
        self
    }

    /// Set security policy
    #[must_use]
    pub fn with_security_policy(mut self, policy: SecurityPolicy) -> Self {
        self.security_policy = policy;
        self
    }

    /// Add custom property
    #[must_use]
    pub fn with_property(mut self, key: String, value: String) -> Self {
        self.properties.insert(key, value);
        self
    }

    /// Start pipeline execution
    pub fn start_execution(&mut self) {
        self.execution_state.is_running = true;
        self.execution_state.start_time = Some(Instant::now());
        self.execution_state.current_stage = ValidationStage::Initialization;
        self.execution_state.progress_percent = 0;
    }

    /// Advance to next stage
    pub fn advance_stage(&mut self, next_stage: ValidationStage) {
        // Record duration for completed stage
        if let Some(start_time) = self.execution_state.start_time {
            let stage_duration = start_time.elapsed();
            self.metrics
                .stage_durations
                .insert(self.execution_state.current_stage.clone(), stage_duration);
        }

        // Mark current stage as completed
        self.execution_state
            .completed_stages
            .push(self.execution_state.current_stage.clone());

        // Advance to next stage
        self.execution_state.current_stage = next_stage;

        // Update progress
        self.execution_state.progress_percent = self.calculate_progress();
    }

    /// Complete pipeline execution
    pub fn complete_execution(&mut self) {
        // Mark current stage as completed if not already done
        if !self
            .execution_state
            .completed_stages
            .contains(&self.execution_state.current_stage)
        {
            self.execution_state
                .completed_stages
                .push(self.execution_state.current_stage.clone());
        }

        self.execution_state.is_running = false;
        self.execution_state.end_time = Some(Instant::now());
        self.execution_state.current_stage = ValidationStage::Completed;
        self.execution_state.progress_percent = 100;

        // Calculate total duration
        if let (Some(start), Some(end)) = (
            self.execution_state.start_time,
            self.execution_state.end_time,
        ) {
            self.metrics.total_duration = Some(end.duration_since(start));
        }
    }

    /// Add warning to metrics
    pub fn add_warning(&mut self) {
        self.metrics.warning_count += 1;
    }

    /// Add error to metrics
    pub fn add_error(&mut self) {
        self.metrics.error_count += 1;
    }

    /// Update files processed count
    pub fn update_files_processed(&mut self, count: usize) {
        self.metrics.files_processed = count;
    }

    /// Update bytes processed count
    pub fn update_bytes_processed(&mut self, bytes: u64) {
        self.metrics.bytes_processed = bytes;
    }

    /// Update peak memory usage
    pub fn update_peak_memory(&mut self, memory_bytes: u64) {
        if memory_bytes > self.metrics.peak_memory_usage {
            self.metrics.peak_memory_usage = memory_bytes;
        }
    }

    /// Calculate current progress percentage
    fn calculate_progress(&self) -> u8 {
        let total_stages = 5; // Init, Format, Content, Security, Finalization
        let completed_count = self.execution_state.completed_stages.len();

        let progress = (completed_count as f32 / total_stages as f32) * 100.0;
        progress.min(100.0) as u8
    }

    /// Get current execution duration
    #[must_use]
    pub fn current_duration(&self) -> Option<Duration> {
        self.execution_state.start_time.map(|start| start.elapsed())
    }

    /// Check if execution has exceeded timeout
    #[must_use]
    pub fn has_exceeded_timeout(&self) -> bool {
        if let Some(current_duration) = self.current_duration() {
            let timeout = Duration::from_secs(self.validation_config.timeout_seconds);
            current_duration > timeout
        } else {
            false
        }
    }

    /// Get execution summary
    #[must_use]
    pub fn execution_summary(&self) -> ExecutionSummary {
        ExecutionSummary {
            total_duration: self.metrics.total_duration,
            stages_completed: self.execution_state.completed_stages.len(),
            files_processed: self.metrics.files_processed,
            bytes_processed: self.metrics.bytes_processed,
            warnings: self.metrics.warning_count,
            errors: self.metrics.error_count,
            peak_memory_mb: self.metrics.peak_memory_usage / (1024 * 1024),
            success: self.execution_state.current_stage == ValidationStage::Completed
                && self.metrics.error_count == 0,
        }
    }

    /// Get stage progress information
    #[must_use]
    pub fn stage_progress(&self) -> StageProgress {
        StageProgress {
            current_stage: self.execution_state.current_stage.clone(),
            progress_percent: self.execution_state.progress_percent,
            completed_stages: self.execution_state.completed_stages.clone(),
            is_running: self.execution_state.is_running,
        }
    }
}

impl Default for PipelineContext {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionState {
    /// Create new execution state
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_stage: ValidationStage::Initialization,
            completed_stages: Vec::new(),
            is_running: false,
            start_time: None,
            end_time: None,
            progress_percent: 0,
        }
    }

    /// Check if stage has been completed
    #[must_use]
    pub fn is_stage_completed(&self, stage: &ValidationStage) -> bool {
        self.completed_stages.contains(stage)
    }

    /// Get human-readable stage name
    #[must_use]
    pub fn stage_name(&self) -> &'static str {
        match self.current_stage {
            ValidationStage::Initialization => "Initializing",
            ValidationStage::FormatValidation => "Validating Format",
            ValidationStage::ContentValidation => "Validating Content",
            ValidationStage::SecurityValidation => "Checking Security",
            ValidationStage::Finalization => "Finalizing",
            ValidationStage::Completed => "Completed",
        }
    }
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self::new()
    }
}

/// Execution summary information
#[derive(Debug, Clone)]
pub struct ExecutionSummary {
    /// Total execution time
    pub total_duration: Option<Duration>,
    /// Number of stages completed
    pub stages_completed: usize,
    /// Number of files processed
    pub files_processed: usize,
    /// Bytes processed
    pub bytes_processed: u64,
    /// Number of warnings
    pub warnings: usize,
    /// Number of errors
    pub errors: usize,
    /// Peak memory usage in MB
    pub peak_memory_mb: u64,
    /// Whether execution was successful
    pub success: bool,
}

/// Stage progress information
#[derive(Debug, Clone)]
pub struct StageProgress {
    /// Current stage
    pub current_stage: ValidationStage,
    /// Progress percentage
    pub progress_percent: u8,
    /// Completed stages
    pub completed_stages: Vec<ValidationStage>,
    /// Whether pipeline is running
    pub is_running: bool,
}

impl StageProgress {
    /// Get human-readable progress description
    #[must_use]
    pub fn description(&self) -> String {
        if !self.is_running {
            if self.current_stage == ValidationStage::Completed {
                "Validation completed".to_string()
            } else {
                "Validation not started".to_string()
            }
        } else {
            format!(
                "{} ({}% complete)",
                match self.current_stage {
                    ValidationStage::Initialization => "Initializing validation",
                    ValidationStage::FormatValidation => "Checking file format",
                    ValidationStage::ContentValidation => "Validating package contents",
                    ValidationStage::SecurityValidation => "Performing security checks",
                    ValidationStage::Finalization => "Finalizing validation",
                    ValidationStage::Completed => "Validation completed",
                },
                self.progress_percent
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_context_creation() {
        let context = PipelineContext::new();
        assert!(!context.execution_state.is_running);
        assert_eq!(
            context.execution_state.current_stage,
            ValidationStage::Initialization
        );
        assert_eq!(context.execution_state.progress_percent, 0);
    }

    #[test]
    fn test_execution_state_operations() {
        let mut context = PipelineContext::new();

        // Start execution
        context.start_execution();
        assert!(context.execution_state.is_running);
        assert!(context.execution_state.start_time.is_some());

        // Advance stage
        context.advance_stage(ValidationStage::FormatValidation);
        assert_eq!(
            context.execution_state.current_stage,
            ValidationStage::FormatValidation
        );
        assert!(context
            .execution_state
            .completed_stages
            .contains(&ValidationStage::Initialization));

        // Complete execution
        context.complete_execution();
        assert!(!context.execution_state.is_running);
        assert_eq!(
            context.execution_state.current_stage,
            ValidationStage::Completed
        );
        assert_eq!(context.execution_state.progress_percent, 100);
    }

    #[test]
    fn test_metrics_tracking() {
        let mut context = PipelineContext::new();

        context.add_warning();
        context.add_error();
        context.update_files_processed(100);
        context.update_bytes_processed(1024 * 1024);
        context.update_peak_memory(512 * 1024 * 1024);

        assert_eq!(context.metrics.warning_count, 1);
        assert_eq!(context.metrics.error_count, 1);
        assert_eq!(context.metrics.files_processed, 100);
        assert_eq!(context.metrics.bytes_processed, 1024 * 1024);
        assert_eq!(context.metrics.peak_memory_usage, 512 * 1024 * 1024);
    }

    #[test]
    fn test_stage_progress() {
        let mut context = PipelineContext::new();
        context.start_execution();

        let progress = context.stage_progress();
        assert!(progress.is_running);
        assert_eq!(progress.current_stage, ValidationStage::Initialization);
        assert_eq!(progress.progress_percent, 0);

        let description = progress.description();
        assert!(description.contains("Initializing"));
    }

    #[test]
    fn test_execution_summary() {
        let mut context = PipelineContext::new();
        context.start_execution();
        context.add_warning();
        context.update_files_processed(50);
        context.complete_execution();

        let summary = context.execution_summary();
        assert!(summary.success); // No errors
        assert_eq!(summary.warnings, 1);
        assert_eq!(summary.files_processed, 50);
        assert_eq!(summary.stages_completed, 1); // Only initialization completed
    }

    #[test]
    fn test_timeout_checking() {
        let mut context = PipelineContext::new();
        context.validation_config.timeout_seconds = 1; // 1 second timeout

        // Should not exceed timeout immediately
        assert!(!context.has_exceeded_timeout());

        context.start_execution();
        // Still should not exceed timeout immediately
        assert!(!context.has_exceeded_timeout());
    }

    #[test]
    fn test_stage_completion_checking() {
        let state = ExecutionState::new();
        assert!(!state.is_stage_completed(&ValidationStage::FormatValidation));

        let mut context = PipelineContext::new();
        context.start_execution();
        context.advance_stage(ValidationStage::FormatValidation);

        assert!(context
            .execution_state
            .is_stage_completed(&ValidationStage::Initialization));
        assert!(!context
            .execution_state
            .is_stage_completed(&ValidationStage::FormatValidation));
    }
}
