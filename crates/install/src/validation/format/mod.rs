//! File format validation module
//!
//! This module provides comprehensive file format validation including:
//! - File extension validation (.sp requirement)
//! - File size validation (empty files, size limits)
//! - Format detection (zstd vs plain tar)
//! - Format support validation

pub mod detection;
pub mod extension;
pub mod size_limits;

use sps2_errors::Error;
use sps2_events::EventSender;
use std::path::Path;

use crate::validation::types::PackageFormat;

pub use detection::{detect_package_format, validate_supported_format};
pub use extension::{
    get_dangerous_extensions, get_extension, has_extension, validate_allowed_extension,
    validate_package_extension,
};
pub use size_limits::{
    format_size, validate_extracted_size, validate_file_size, validate_memory_size,
};

/// Validates file format (extension, size, magic bytes)
///
/// This is the main entry point for file format validation. It performs
/// all format-related checks in sequence:
///
/// 1. Extension validation (.sp requirement)
/// 2. File size validation (not empty, within limits)
/// 3. Format detection (zstd vs tar)
/// 4. Format support validation
///
/// # Errors
///
/// Returns an error if any format validation step fails.
pub async fn validate_file_format(
    file_path: &Path,
    event_sender: Option<&EventSender>,
) -> Result<PackageFormat, Error> {
    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::DebugLog {
            message: format!(
                "DEBUG: validate_file_format - checking extension for: {}",
                file_path.display()
            ),
            context: std::collections::HashMap::new(),
        });
    }

    // Step 1: Check file extension
    validate_package_extension(file_path)?;

    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::DebugLog {
            message: "DEBUG: validate_file_format - extension check passed".to_string(),
            context: std::collections::HashMap::new(),
        });
    }

    // Step 2: Check file size
    let _file_size = validate_file_size(file_path).await?;

    // Step 3: Detect format by reading magic bytes
    let format = detect_package_format(file_path).await?;

    // Step 4: Validate format is supported
    validate_supported_format(&format)?;

    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::OperationCompleted {
            operation: "File format validation completed".to_string(),
            success: true,
        });
    }

    Ok(format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_validate_file_format_invalid_extension() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"test content").unwrap();

        // Rename to invalid extension
        let invalid_path = temp_file.path().with_extension("txt");
        std::fs::copy(temp_file.path(), &invalid_path).unwrap();

        let result = validate_file_format(&invalid_path, None).await;
        assert!(result.is_err());

        if let Err(e) = result {
            let error_message = e.to_string();
            assert!(error_message.contains("invalid file extension"));
        }

        // Clean up
        let _ = std::fs::remove_file(invalid_path);
    }

    #[tokio::test]
    async fn test_validate_file_format_empty() {
        let temp_file = NamedTempFile::with_suffix(".sp").unwrap();
        // File is created empty by default

        let result = validate_file_format(temp_file.path(), None).await;
        assert!(result.is_err());

        if let Err(e) = result {
            let error_message = e.to_string();
            assert!(error_message.contains("file is empty"));
        }
    }
}
