//! Content validation module
//!
//! This module provides comprehensive validation of package content including:
//! - Tar archive validation with corruption recovery
//! - Zstd compression validation and decompression testing
//! - Manifest.toml validation and parsing
//! - Content limits enforcement (file counts, sizes, depths)

pub mod limits;
pub mod manifest;
pub mod tar;
pub mod zstd;

use sps2_errors::Error;
use sps2_events::EventSender;
use std::path::Path;

use crate::validation::types::{PackageFormat, ValidationResult};

pub use limits::{
    validate_file_count, validate_individual_file_size, validate_path_depth, validate_path_length,
    validate_total_extracted_size, ContentLimits, ContentStats,
};
pub use manifest::{validate_manifest_content, ManifestValidation};
pub use tar::{validate_tar_archive_content, validate_tar_entry_safety};
pub use zstd::{test_zstd_decompression, validate_zstd_archive_content, validate_zstd_parameters};

/// Validates archive content without full extraction
///
/// This is the main entry point for content validation. It dispatches
/// to the appropriate validation function based on the package format
/// and handles error recovery for corrupted archives.
///
/// # Errors
///
/// Returns an error if:
/// - Package format is unknown/unsupported
/// - Content validation fails critically
/// - Archive is too corrupted to process
pub async fn validate_archive_content(
    file_path: &Path,
    format: &PackageFormat,
    result: &mut ValidationResult,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::OperationStarted {
            operation: "Validating archive content".to_string(),
        });
    }

    match format {
        PackageFormat::ZstdCompressed => {
            validate_zstd_archive_content(file_path, result).await?;
        }
        PackageFormat::PlainTar => {
            validate_tar_archive_content(file_path, result).await?;
        }
        PackageFormat::Unknown => {
            return Err(sps2_errors::InstallError::InvalidPackageFile {
                path: file_path.display().to_string(),
                message: "unknown package format".to_string(),
            }
            .into());
        }
    }

    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::OperationCompleted {
            operation: "Archive content validation completed".to_string(),
            success: true,
        });
    }

    Ok(())
}

/// Validates package manifest if present in results
///
/// This function checks if the validation results contain a manifest
/// and validates its content if present.
pub fn validate_package_manifest(result: &mut ValidationResult) -> Result<(), Error> {
    if let Some(manifest_content) = &result.manifest {
        match validate_manifest_content(manifest_content) {
            Ok(validation) => {
                // Add warnings from manifest validation
                for warning in validation.warnings {
                    result.add_warning(format!("manifest: {warning}"));
                }
            }
            Err(e) => {
                // Add manifest errors as warnings since we already have the content
                result.add_warning(format!("manifest validation failed: {e}"));
            }
        }
    } else {
        result.add_warning("manifest.toml not found or not readable".to_string());
    }

    Ok(())
}

/// Validates content against specified limits
///
/// This function checks the validation results against content limits
/// to ensure the package doesn't exceed resource constraints.
pub fn validate_content_limits(
    result: &ValidationResult,
    limits: &ContentLimits,
) -> Result<(), Error> {
    limits.validate_totals(result.file_count, result.extracted_size)?;
    Ok(())
}

/// Comprehensive content validation
///
/// This function performs all content validation steps including format-specific
/// validation, manifest checking, and limits enforcement.
pub async fn validate_content_comprehensive(
    file_path: &Path,
    format: &PackageFormat,
    result: &mut ValidationResult,
    limits: &ContentLimits,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    // Step 1: Format-specific content validation
    validate_archive_content(file_path, format, result, event_sender).await?;

    // Step 2: Manifest validation
    validate_package_manifest(result)?;

    // Step 3: Content limits validation
    validate_content_limits(result, limits)?;

    Ok(())
}
