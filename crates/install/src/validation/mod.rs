//! Package validation module for sps2
//!
//! This module provides comprehensive validation of .sp package files to ensure
//! they are safe and well-formed before extraction. The validation system is
//! organized into several specialized modules:
//!
//! - **format**: File format validation (extension, size, magic bytes)
//! - **content**: Archive content validation (tar, zstd, manifest)
//! - **security**: Security validation (paths, permissions, symlinks)
//! - **pipeline**: Validation orchestration and error recovery
//! - **types**: Shared types and constants
//!
//! # Main Entry Points
//!
//! - [`validate_sp_file`] - Main validation function with default settings
//! - [`pipeline::quick_validate`] - Fast validation for development
//! - [`pipeline::strict_validate`] - Strict validation for production
//! - [`validate_tar_archive_content`] - Direct tar archive validation
//!
//! # Examples
//!
//! ```rust,no_run
//! use std::path::Path;
//! use sps2_install::validation::validate_sp_file;
//!
//! async fn validate_package() -> Result<(), Box<dyn std::error::Error>> {
//!     let path = Path::new("package.sp");
//!     let result = validate_sp_file(&path, None).await?;
//!     
//!     if result.is_valid {
//!         println!("Package is valid with {} files", result.file_count);
//!     } else {
//!         println!("Package validation failed");
//!     }
//!     
//!     for warning in &result.warnings {
//!         println!("Warning: {}", warning);
//!     }
//!     
//!     Ok(())
//! }
//! ```

pub mod content;
pub mod format;
pub mod pipeline;
pub mod security;
pub mod types;

// Re-export main types and functions for convenience
pub use pipeline::validate_sp_file;
pub use types::{PackageFormat, ValidationContext, ValidationResult};

// Re-export key validation functions
pub use content::validate_tar_archive_content;
pub use format::validate_file_format;
pub use security::validate_security_properties;

// Re-export pipeline components
pub use pipeline::{
    quick_validate, strict_validate, validate_batch, validate_with_context, ErrorRecoveryManager,
    RecoveryStrategy, ValidationOrchestrator, ValidationPipelineBuilder,
};

// Re-export useful types from submodules
pub use content::{ContentLimits, ManifestValidation};
pub use security::{SecurityLevel, SecurityPolicy, SecurityReport};
pub use types::SecurityPolicy as ValidationSecurityPolicy;

use sps2_errors::Error;
use sps2_events::EventSender;
use std::path::Path;

/// Validates a .sp package file comprehensively
///
/// This function performs all validation checks:
/// 1. File format validation (extension, size, magic bytes)
/// 2. Archive content validation (manifest presence, structure)
/// 3. Security validation (path traversal, symlinks, permissions)
///
/// This is the main entry point for package validation and uses sensible
/// defaults with error recovery enabled.
///
/// # Arguments
///
/// * `file_path` - Path to the .sp package file
/// * `event_sender` - Optional event sender for progress reporting
///
/// # Returns
///
/// Returns a `ValidationResult` containing detailed validation information
/// including file counts, extracted sizes, warnings, and manifest content.
///
/// # Errors
///
/// Returns an error if:
/// - File cannot be accessed or read
/// - File format is invalid or unsupported
/// - Archive structure is malformed
/// - Security validation fails critically
/// - Content validation fails critically
///
/// # Examples
///
/// ```rust,no_run
/// use std::path::Path;
/// use sps2_install::validation::validate_sp_file;
///
/// async fn example() -> Result<(), Box<dyn std::error::Error>> {
///     let package_path = Path::new("example.sp");
///     let result = validate_sp_file(&package_path, None).await?;
///     
///     println!("Package format: {:?}", result.format);
///     println!("File count: {}", result.file_count);
///     println!("Extracted size: {} bytes", result.extracted_size);
///     
///     if !result.warnings.is_empty() {
///         println!("Warnings:");
///         for warning in &result.warnings {
///             println!("  - {}", warning);
///         }
///     }
///     
///     Ok(())
/// }
/// ```
pub async fn validate_sp_file_comprehensive(
    file_path: &Path,
    event_sender: Option<&EventSender>,
) -> Result<ValidationResult, Error> {
    pipeline::validate_sp_file(file_path, event_sender).await
}

/// Validates a package with custom security policy
///
/// This function allows specifying a custom security policy for validation,
/// useful for environments with specific security requirements.
pub async fn validate_with_security_policy(
    file_path: &Path,
    security_policy: SecurityPolicy,
    event_sender: Option<&EventSender>,
) -> Result<ValidationResult, Error> {
    let orchestrator = ValidationOrchestrator::new()
        .with_security_policy(security_policy)
        .with_continue_on_errors(true);

    orchestrator.validate_package(file_path, event_sender).await
}

/// Validates a package with custom content limits
///
/// This function allows specifying custom limits for package content,
/// useful for controlling resource usage during validation.
pub async fn validate_with_content_limits(
    file_path: &Path,
    content_limits: ContentLimits,
    event_sender: Option<&EventSender>,
) -> Result<ValidationResult, Error> {
    let orchestrator = ValidationOrchestrator::new()
        .with_content_limits(content_limits)
        .with_continue_on_errors(true);

    orchestrator.validate_package(file_path, event_sender).await
}

/// Validates package format only (lightweight check)
///
/// This function performs only format validation without content inspection,
/// useful for quick checks or when full validation is not needed.
pub async fn validate_format_only(
    file_path: &Path,
    event_sender: Option<&EventSender>,
) -> Result<PackageFormat, Error> {
    format::validate_file_format(file_path, event_sender).await
}

/// Creates a validation report with detailed analysis
///
/// This function performs validation and creates a comprehensive report
/// suitable for logging, auditing, or user display.
pub async fn create_validation_report(
    file_path: &Path,
    event_sender: Option<&EventSender>,
) -> Result<ValidationReport, Error> {
    let start_time = std::time::Instant::now();
    let result = validate_sp_file(file_path, event_sender).await?;
    let duration = start_time.elapsed();

    Ok(ValidationReport {
        file_path: file_path.display().to_string(),
        validation_result: result,
        validation_duration: duration,
        timestamp: std::time::SystemTime::now(),
    })
}

/// Comprehensive validation report
#[derive(Debug)]
pub struct ValidationReport {
    /// Path to the validated file
    pub file_path: String,
    /// Validation result
    pub validation_result: ValidationResult,
    /// Time taken for validation
    pub validation_duration: std::time::Duration,
    /// Timestamp when validation was performed
    pub timestamp: std::time::SystemTime,
}

impl ValidationReport {
    /// Create a summary of the validation report
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Validation of '{}' - {} in {:.2}s - {} files, {} warnings",
            self.file_path,
            if self.validation_result.is_valid {
                "VALID"
            } else {
                "INVALID"
            },
            self.validation_duration.as_secs_f64(),
            self.validation_result.file_count,
            self.validation_result.warnings.len()
        )
    }

    /// Check if validation was successful
    #[must_use]
    pub fn is_successful(&self) -> bool {
        self.validation_result.is_valid
    }

    /// Get formatted warnings
    #[must_use]
    pub fn formatted_warnings(&self) -> Vec<String> {
        self.validation_result
            .warnings
            .iter()
            .enumerate()
            .map(|(i, warning)| format!("  {}. {}", i + 1, warning))
            .collect()
    }
}

/// Validation configuration presets
pub struct ValidationPresets;

impl ValidationPresets {
    /// Development validation preset (permissive)
    #[must_use]
    pub fn development() -> ValidationPipelineBuilder {
        ValidationPipelineBuilder::new()
            .with_recovery_strategy(RecoveryStrategy::ContinueWithWarnings)
            .with_security_policy(SecurityPolicy::permissive())
            .with_timeout(600) // 10 minutes
    }

    /// Production validation preset (balanced)
    #[must_use]
    pub fn production() -> ValidationPipelineBuilder {
        ValidationPipelineBuilder::new()
            .with_recovery_strategy(RecoveryStrategy::AutoRecover)
            .with_security_policy(SecurityPolicy::standard())
            .with_timeout(300) // 5 minutes
    }

    /// High-security validation preset (strict)
    #[must_use]
    pub fn high_security() -> ValidationPipelineBuilder {
        ValidationPipelineBuilder::new()
            .with_recovery_strategy(RecoveryStrategy::FailFast)
            .with_security_policy(SecurityPolicy::strict())
            .with_content_limits(
                ContentLimits::new()
                    .with_max_files(5000)
                    .with_max_file_size(10 * 1024 * 1024), // 10MB
            )
            .with_timeout(120) // 2 minutes
    }

    /// CI/CD validation preset (fast)
    #[must_use]
    pub fn ci_cd() -> ValidationPipelineBuilder {
        ValidationPipelineBuilder::new()
            .with_recovery_strategy(RecoveryStrategy::ContinueWithWarnings)
            .with_security_policy(SecurityPolicy::standard())
            .with_timeout(60) // 1 minute
    }
}
