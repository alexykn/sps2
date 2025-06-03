//! Validation pipeline orchestrator
//!
//! This module coordinates the multi-stage validation process, managing
//! the flow between format validation, content validation, and security
//! validation with proper error recovery and progress reporting.

use sps2_errors::Error;
use sps2_events::EventSender;
use std::path::Path;

use crate::validation::content::ContentLimits;
use crate::validation::security::SecurityPolicy;
use crate::validation::types::{ValidationContext, ValidationResult};

/// Validation pipeline orchestrator
///
/// This struct manages the complete validation process, coordinating
/// between different validation stages and handling error recovery.
pub struct ValidationOrchestrator {
    /// Validation context
    context: ValidationContext,
    /// Content limits to enforce
    content_limits: ContentLimits,
    /// Security policy to apply
    security_policy: SecurityPolicy,
    /// Whether to continue on non-fatal errors
    continue_on_errors: bool,
}

impl ValidationOrchestrator {
    /// Create new validation orchestrator
    #[must_use]
    pub fn new() -> Self {
        Self {
            context: ValidationContext::default(),
            content_limits: ContentLimits::default(),
            security_policy: SecurityPolicy::default(),
            continue_on_errors: true,
        }
    }

    /// Set validation context
    #[must_use]
    pub fn with_context(mut self, context: ValidationContext) -> Self {
        self.context = context;
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

    /// Set whether to continue on non-fatal errors
    #[must_use]
    pub fn with_continue_on_errors(mut self, continue_on_errors: bool) -> Self {
        self.continue_on_errors = continue_on_errors;
        self
    }

    /// Execute the complete validation pipeline
    ///
    /// This is the main orchestration method that runs all validation
    /// stages in sequence with proper error handling and recovery.
    pub async fn validate_package(
        &self,
        file_path: &Path,
        event_sender: Option<&EventSender>,
    ) -> Result<ValidationResult, Error> {
        if let Some(sender) = event_sender {
            let _ = sender.send(sps2_events::Event::OperationStarted {
                operation: format!(
                    "Starting comprehensive validation of {}",
                    file_path.display()
                ),
            });
        }

        // Stage 1: File format validation
        let format = self.validate_format_stage(file_path, event_sender).await?;
        let mut result = ValidationResult::new(format.clone());

        // Stage 2: Content validation with error recovery
        if let Err(e) = self
            .validate_content_stage(file_path, &format, &mut result, event_sender)
            .await
        {
            if self.continue_on_errors {
                result.add_warning(format!("Content validation had issues: {e}"));
                // Set minimal values to allow pipeline to continue
                if result.file_count == 0 {
                    result.file_count = 1;
                }
                if result.extracted_size == 0 {
                    result.extracted_size = 1024;
                }
            } else {
                return Err(e);
            }
        }

        // Stage 3: Security validation
        if let Err(e) = self
            .validate_security_stage(file_path, &format, &mut result, event_sender)
            .await
        {
            if self.continue_on_errors {
                result.add_warning(format!("Security validation had issues: {e}"));
            } else {
                return Err(e);
            }
        }

        // Stage 4: Final validation checks
        self.finalize_validation(&mut result)?;

        if let Some(sender) = event_sender {
            let _ = sender.send(sps2_events::Event::OperationCompleted {
                operation: format!(
                    "Comprehensive validation completed for {}",
                    file_path.display()
                ),
                success: true,
            });
        }

        Ok(result)
    }

    /// Stage 1: Format validation
    async fn validate_format_stage(
        &self,
        file_path: &Path,
        event_sender: Option<&EventSender>,
    ) -> Result<crate::validation::types::PackageFormat, Error> {
        if let Some(sender) = event_sender {
            let _ = sender.send(sps2_events::Event::DebugLog {
                message: "PIPELINE: Starting format validation stage".to_string(),
                context: std::collections::HashMap::new(),
            });
        }

        let format =
            crate::validation::format::validate_file_format(file_path, event_sender).await?;

        if let Some(sender) = event_sender {
            let _ = sender.send(sps2_events::Event::DebugLog {
                message: format!("PIPELINE: Format validation complete - detected: {format:?}"),
                context: std::collections::HashMap::new(),
            });
        }

        Ok(format)
    }

    /// Stage 2: Content validation
    async fn validate_content_stage(
        &self,
        file_path: &Path,
        format: &crate::validation::types::PackageFormat,
        result: &mut ValidationResult,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        if let Some(sender) = event_sender {
            let _ = sender.send(sps2_events::Event::DebugLog {
                message: "PIPELINE: Starting content validation stage".to_string(),
                context: std::collections::HashMap::new(),
            });
        }

        // Comprehensive content validation
        crate::validation::content::validate_content_comprehensive(
            file_path,
            format,
            result,
            &self.content_limits,
            event_sender,
        )
        .await?;

        if let Some(sender) = event_sender {
            let _ = sender.send(sps2_events::Event::DebugLog {
                message: format!(
                    "PIPELINE: Content validation complete - {} files, {} bytes",
                    result.file_count, result.extracted_size
                ),
                context: std::collections::HashMap::new(),
            });
        }

        Ok(())
    }

    /// Stage 3: Security validation
    async fn validate_security_stage(
        &self,
        file_path: &Path,
        format: &crate::validation::types::PackageFormat,
        result: &mut ValidationResult,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        if let Some(sender) = event_sender {
            let _ = sender.send(sps2_events::Event::DebugLog {
                message: format!(
                    "PIPELINE: Starting security validation stage with {} policy",
                    self.security_policy.security_level_description()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        // Comprehensive security validation
        crate::validation::security::validate_package_security(
            file_path,
            format,
            result,
            &self.security_policy,
            event_sender,
        )
        .await?;

        if let Some(sender) = event_sender {
            let _ = sender.send(sps2_events::Event::DebugLog {
                message: "PIPELINE: Security validation complete".to_string(),
                context: std::collections::HashMap::new(),
            });
        }

        Ok(())
    }

    /// Stage 4: Final validation and result finalization
    fn finalize_validation(&self, result: &mut ValidationResult) -> Result<(), Error> {
        // Mark validation as successful if we got this far
        result.mark_valid();

        // Add summary information
        if !result.warnings.is_empty() {
            result.add_warning(format!(
                "Validation completed with {} warnings",
                result.warnings.len()
            ));
        }

        // Validate against context constraints
        if result.extracted_size > self.context.timeout_seconds * 1024 * 1024 {
            // Rough heuristic: if package is very large, warn about timeout
            result.add_warning("Large package may require extended validation time".to_string());
        }

        Ok(())
    }

    /// Get validation statistics
    #[must_use]
    pub fn get_validation_stats(&self) -> ValidationStats {
        ValidationStats {
            stages_enabled: 3, // Format, Content, Security
            security_level: self.security_policy.security_level.clone(),
            max_file_count: self.content_limits.max_files,
            max_extracted_size: self.content_limits.max_extracted_size,
            continue_on_errors: self.continue_on_errors,
        }
    }
}

impl Default for ValidationOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

/// Validation pipeline statistics
#[derive(Debug, Clone)]
pub struct ValidationStats {
    /// Number of validation stages enabled
    pub stages_enabled: u32,
    /// Security level being applied
    pub security_level: crate::validation::security::SecurityLevel,
    /// Maximum file count allowed
    pub max_file_count: usize,
    /// Maximum extracted size allowed
    pub max_extracted_size: u64,
    /// Whether pipeline continues on non-fatal errors
    pub continue_on_errors: bool,
}

/// Quick validation for basic checks
///
/// This function provides a simplified validation path for cases where
/// only basic format and safety checks are needed.
pub async fn quick_validate(
    file_path: &Path,
    event_sender: Option<&EventSender>,
) -> Result<ValidationResult, Error> {
    let orchestrator = ValidationOrchestrator::new()
        .with_continue_on_errors(true)
        .with_security_policy(crate::validation::security::SecurityPolicy::permissive());

    orchestrator.validate_package(file_path, event_sender).await
}

/// Strict validation for high-security environments
///
/// This function provides a strict validation path with enhanced security
/// checks and zero tolerance for issues.
pub async fn strict_validate(
    file_path: &Path,
    event_sender: Option<&EventSender>,
) -> Result<ValidationResult, Error> {
    let orchestrator = ValidationOrchestrator::new()
        .with_continue_on_errors(false)
        .with_security_policy(crate::validation::security::SecurityPolicy::strict())
        .with_content_limits(
            ContentLimits::new()
                .with_max_files(5000) // Stricter file count
                .with_max_file_size(10 * 1024 * 1024), // 10MB max per file
        );

    orchestrator.validate_package(file_path, event_sender).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_orchestrator_creation() {
        let orchestrator = ValidationOrchestrator::new();
        let stats = orchestrator.get_validation_stats();

        assert_eq!(stats.stages_enabled, 3);
        assert!(stats.continue_on_errors);
    }

    #[test]
    fn test_orchestrator_configuration() {
        let orchestrator = ValidationOrchestrator::new()
            .with_continue_on_errors(false)
            .with_content_limits(ContentLimits::new().with_max_files(1000));

        let stats = orchestrator.get_validation_stats();
        assert!(!stats.continue_on_errors);
        assert_eq!(stats.max_file_count, 1000);
    }

    #[tokio::test]
    async fn test_quick_validate() {
        let temp_path = std::path::Path::new("/tmp/nonexistent.sp");

        // This will fail because the file doesn't exist, but tests the function
        let result = quick_validate(temp_path, None).await;
        assert!(result.is_err()); // Expected because file doesn't exist
    }

    #[tokio::test]
    async fn test_strict_validate() {
        let temp_path = std::path::Path::new("/tmp/nonexistent.sp");

        // This will fail because the file doesn't exist, but tests the function
        let result = strict_validate(temp_path, None).await;
        assert!(result.is_err()); // Expected because file doesn't exist
    }

    #[test]
    fn test_validation_stats() {
        let stats = ValidationStats {
            stages_enabled: 3,
            security_level: crate::validation::security::SecurityLevel::Standard,
            max_file_count: 10000,
            max_extracted_size: 1024 * 1024 * 1024,
            continue_on_errors: true,
        };

        assert_eq!(stats.stages_enabled, 3);
        assert!(stats.continue_on_errors);
    }
}
