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
use sps2_events::{EventEmitter, EventSender};
use std::path::Path;

use crate::validation::types::ValidationResult;

pub use context::{ExecutionState, ExecutionSummary, PipelineContext, PipelineMetrics};
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
        let () = sender.emit(sps2_events::AppEvent::Install(
            sps2_events::InstallEvent::ValidationStarted {
                package: "unknown".to_string(), // TODO: Extract package name from file_path
                version: sps2_types::Version::new(0, 0, 0), // TODO: Extract version from file_path
                validation_checks: vec![
                    "format".to_string(),
                    "content".to_string(),
                    "security".to_string(),
                ],
            },
        ));
        let () = sender.emit(sps2_events::AppEvent::General(
            sps2_events::GeneralEvent::DebugLog {
                message: format!("DEBUG: Starting validation of {}", file_path.display()),
                context: std::collections::HashMap::new(),
            },
        ));
    }

    // Create orchestrator with default settings
    let orchestrator = ValidationOrchestrator::new().with_continue_on_errors(true); // Enable error recovery by default

    // Execute validation pipeline
    let result = orchestrator
        .validate_package(file_path, event_sender)
        .await?;

    if let Some(sender) = event_sender {
        let () = sender.emit(sps2_events::AppEvent::General(
            sps2_events::GeneralEvent::OperationCompleted {
                operation: format!("Validation completed for {}", file_path.display()),
                success: true,
            },
        ));
        let () = sender.emit(sps2_events::AppEvent::General(
            sps2_events::GeneralEvent::DebugLog {
                message: "PIPELINE: Starting format validation stage".to_string(),
                context: std::collections::HashMap::new(),
            },
        ));
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

    for file_path in file_paths {
        let file_name = file_path.display().to_string();
        let result = orchestrator.validate_package(file_path, event_sender).await;

        if let Some(sender) = event_sender {
            if let Ok(ref _validation_result) = result {
                let () = sender.emit(sps2_events::AppEvent::General(
                    sps2_events::GeneralEvent::DebugLog {
                        message: format!(
                            "PIPELINE: Content validation complete for {} - validation passed",
                            file_name
                        ),
                        context: std::collections::HashMap::new(),
                    },
                ));
            }
        }
        results.push((file_name, result));
    }

    Ok(results)
}
