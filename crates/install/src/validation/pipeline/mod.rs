//! Validation pipeline module
//!
//! This module provides the validation pipeline infrastructure including:
//! - Multi-stage validation orchestration
//! - Pipeline context and state management
//! - Error recovery and resilience mechanisms

pub mod context;
pub mod orchestrator;
pub mod recovery;

use sps2_errors::Error;
use sps2_events::EventSender;
use std::path::Path;

use crate::validation::types::ValidationResult;

pub use context::{
    ExecutionState, ExecutionSummary, PipelineContext, PipelineMetrics, StageProgress,
    ValidationStage,
};
pub use orchestrator::{quick_validate, strict_validate, ValidationOrchestrator, ValidationStats};
pub use recovery::{
    resilient_validation, ErrorRecoveryManager, RecoveryAction, RecoveryPresets, RecoveryStats,
    RecoveryStrategy,
};

/// Main validation pipeline entry point
///
/// This function provides the primary interface for package validation,
/// using a comprehensive pipeline with error recovery and progress reporting.
///
/// # Arguments
///
/// * `file_path` - Path to the .sp package file to validate
/// * `event_sender` - Optional event sender for progress reporting
///
/// # Returns
///
/// Returns a `ValidationResult` containing detailed information about
/// the validation process and any issues found.
///
/// # Errors
///
/// Returns an error if validation fails critically or if the package
/// is determined to be unsafe for installation.
pub async fn validate_sp_file(
    file_path: &Path,
    event_sender: Option<&EventSender>,
) -> Result<ValidationResult, Error> {
    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::OperationStarted {
            operation: format!("Validating package {}", file_path.display()),
        });
        let _ = sender.send(sps2_events::Event::DebugLog {
            message: format!("DEBUG: Starting validation of {}", file_path.display()),
            context: std::collections::HashMap::new(),
        });
    }

    // Create orchestrator with default settings
    let orchestrator = ValidationOrchestrator::new().with_continue_on_errors(true); // Enable error recovery by default

    // Execute validation pipeline
    let result = orchestrator
        .validate_package(file_path, event_sender)
        .await?;

    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::OperationCompleted {
            operation: format!("Package validation completed: {}", file_path.display()),
            success: true,
        });
        let _ = sender.send(sps2_events::Event::DebugLog {
            message: "DEBUG: Validation completed successfully".to_string(),
            context: std::collections::HashMap::new(),
        });
    }

    Ok(result)
}

/// Validates a package with custom pipeline configuration
///
/// This function allows for more control over the validation process
/// by accepting a custom pipeline context with specific settings.
pub async fn validate_with_context(
    file_path: &Path,
    context: PipelineContext,
    event_sender: Option<&EventSender>,
) -> Result<(ValidationResult, ExecutionSummary), Error> {
    let orchestrator = ValidationOrchestrator::new()
        .with_context(context.validation_config.clone())
        .with_content_limits(context.content_limits.clone())
        .with_security_policy(context.security_policy.clone());

    let result = orchestrator
        .validate_package(file_path, event_sender)
        .await?;
    let summary = context.execution_summary();

    Ok((result, summary))
}

/// Validates a package with progress tracking
///
/// This function provides detailed progress information during validation,
/// useful for long-running validations or user interfaces.
pub async fn validate_with_progress<F>(
    file_path: &Path,
    event_sender: Option<&EventSender>,
    progress_callback: F,
) -> Result<ValidationResult, Error>
where
    F: Fn(StageProgress) + Send + Sync,
{
    let mut context = PipelineContext::new();
    context.start_execution();

    // Create a custom event sender that calls our progress callback
    let _progress_sender = if let Some(sender) = event_sender {
        Some(ProgressTrackingEventSender::new(sender, progress_callback))
    } else {
        None
    };

    let orchestrator = ValidationOrchestrator::new();

    // We'd need to modify the orchestrator to accept the progress context
    // For now, we'll use the standard validation
    orchestrator.validate_package(file_path, event_sender).await
}

/// Event sender wrapper that tracks progress
#[allow(dead_code)]
struct ProgressTrackingEventSender<'a, F> {
    #[allow(dead_code)]
    inner: &'a EventSender,
    #[allow(dead_code)]
    progress_callback: F,
    #[allow(dead_code)]
    current_stage: ValidationStage,
}

impl<'a, F> ProgressTrackingEventSender<'a, F>
where
    F: Fn(StageProgress),
{
    fn new(inner: &'a EventSender, progress_callback: F) -> Self {
        Self {
            inner,
            progress_callback,
            current_stage: ValidationStage::Initialization,
        }
    }
}

/// Validation pipeline builder
///
/// This builder provides a fluent interface for configuring and
/// executing validation pipelines with specific requirements.
pub struct ValidationPipelineBuilder {
    context: PipelineContext,
    recovery_strategy: RecoveryStrategy,
    enable_progress_tracking: bool,
}

impl ValidationPipelineBuilder {
    /// Create new pipeline builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            context: PipelineContext::new(),
            recovery_strategy: RecoveryStrategy::ContinueWithWarnings,
            enable_progress_tracking: false,
        }
    }

    /// Set validation timeout
    #[must_use]
    pub fn with_timeout(mut self, timeout_seconds: u64) -> Self {
        self.context.validation_config.timeout_seconds = timeout_seconds;
        self
    }

    /// Set content limits
    #[must_use]
    pub fn with_content_limits(
        mut self,
        limits: crate::validation::content::ContentLimits,
    ) -> Self {
        self.context.content_limits = limits;
        self
    }

    /// Set security policy
    #[must_use]
    pub fn with_security_policy(
        mut self,
        policy: crate::validation::security::SecurityPolicy,
    ) -> Self {
        self.context.security_policy = policy;
        self
    }

    /// Set error recovery strategy
    #[must_use]
    pub fn with_recovery_strategy(mut self, strategy: RecoveryStrategy) -> Self {
        self.recovery_strategy = strategy;
        self
    }

    /// Enable progress tracking
    #[must_use]
    pub fn with_progress_tracking(mut self, enable: bool) -> Self {
        self.enable_progress_tracking = enable;
        self
    }

    /// Build and execute the validation pipeline
    pub async fn validate(
        self,
        file_path: &Path,
        event_sender: Option<&EventSender>,
    ) -> Result<ValidationResult, Error> {
        let orchestrator = ValidationOrchestrator::new()
            .with_context(self.context.validation_config)
            .with_content_limits(self.context.content_limits)
            .with_security_policy(self.context.security_policy)
            .with_continue_on_errors(matches!(
                self.recovery_strategy,
                RecoveryStrategy::ContinueWithWarnings | RecoveryStrategy::AutoRecover
            ));

        orchestrator.validate_package(file_path, event_sender).await
    }
}

impl Default for ValidationPipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Batch validation for multiple packages
///
/// This function validates multiple packages efficiently, with shared
/// configuration and progress reporting.
pub async fn validate_batch(
    file_paths: &[&Path],
    event_sender: Option<&EventSender>,
) -> Result<Vec<(String, Result<ValidationResult, Error>)>, Error> {
    let mut results = Vec::new();
    let orchestrator = ValidationOrchestrator::new();

    for (index, file_path) in file_paths.iter().enumerate() {
        if let Some(sender) = event_sender {
            let _ = sender.send(sps2_events::Event::DebugLog {
                message: format!(
                    "BATCH: Validating package {} of {} - {}",
                    index + 1,
                    file_paths.len(),
                    file_path.display()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        let file_name = file_path.display().to_string();
        let result = orchestrator.validate_package(file_path, event_sender).await;
        results.push((file_name, result));
    }

    Ok(results)
}
