//! Size validation for package files
//!
//! This module handles validation of file sizes to ensure they are within
//! acceptable limits for processing and storage.

use sps2_errors::{Error, InstallError};
use std::path::Path;

use crate::validation::types::{MAX_EXTRACTED_SIZE, MAX_PACKAGE_SIZE};

/// Validates that a file size is within acceptable limits
///
/// This function checks that the package file is not empty and doesn't
/// exceed the maximum allowed size for package files.
///
/// # Errors
///
/// Returns an error if:
/// - File metadata cannot be accessed
/// - File is empty (0 bytes)
/// - File exceeds maximum allowed size
pub async fn validate_file_size(file_path: &Path) -> Result<u64, Error> {
    let metadata =
        tokio::fs::metadata(file_path)
            .await
            .map_err(|e| InstallError::InvalidPackageFile {
                path: file_path.display().to_string(),
                message: format!("cannot access file: {e}"),
            })?;

    let file_size = metadata.len();

    if file_size == 0 {
        return Err(InstallError::InvalidPackageFile {
            path: file_path.display().to_string(),
            message: "file is empty".to_string(),
        }
        .into());
    }

    if file_size > MAX_PACKAGE_SIZE {
        return Err(InstallError::InvalidPackageFile {
            path: file_path.display().to_string(),
            message: format!("file too large: {file_size} bytes (max: {MAX_PACKAGE_SIZE} bytes)"),
        }
        .into());
    }

    Ok(file_size)
}

/// Validates that extracted content size is within limits
///
/// This function checks that the total size of content that would be
/// extracted from the package doesn't exceed storage limits.
pub fn validate_extracted_size(extracted_size: u64) -> Result<(), Error> {
    if extracted_size > MAX_EXTRACTED_SIZE {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!(
                "extracted content too large: {extracted_size} bytes (max: {MAX_EXTRACTED_SIZE} bytes)"
            ),
        }
        .into());
    }
    Ok(())
}

/// Validates that a file size is reasonable for in-memory processing
///
/// Some operations may need to load file contents into memory, so this
/// validates that the size is manageable.
pub fn validate_memory_size(size: u64, max_memory_mb: u64) -> Result<(), Error> {
    let max_memory_bytes = max_memory_mb * 1024 * 1024;
    if size > max_memory_bytes {
        return Err(InstallError::InvalidPackageFile {
            path: "file".to_string(),
            message: format!(
                "file too large for memory processing: {size} bytes (max: {max_memory_bytes} bytes)"
            ),
        }
        .into());
    }
    Ok(())
}

/// Gets human-readable size formatting
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{size:.0} {}", UNITS[unit_index])
    } else {
        format!("{size:.1} {}", UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_validate_file_size_empty() {
        let temp_file = NamedTempFile::with_suffix(".sp").unwrap();
        // File is created empty by default

        let result = validate_file_size(temp_file.path()).await;
        assert!(result.is_err());

        if let Err(e) = result {
            let error_message = e.to_string();
            assert!(error_message.contains("file is empty"));
        }
    }

    #[tokio::test]
    async fn test_validate_file_size_valid() {
        let mut temp_file = NamedTempFile::with_suffix(".sp").unwrap();
        temp_file.write_all(b"test content").unwrap();

        let result = validate_file_size(temp_file.path()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 12); // "test content" is 12 bytes
    }

    #[test]
    fn test_validate_extracted_size() {
        // Valid size
        assert!(validate_extracted_size(1024 * 1024).is_ok());

        // Too large
        assert!(validate_extracted_size(MAX_EXTRACTED_SIZE + 1).is_err());
    }

    #[test]
    fn test_validate_memory_size() {
        // Valid size (1MB limit)
        assert!(validate_memory_size(512 * 1024, 1).is_ok());

        // Too large (1MB limit)
        assert!(validate_memory_size(2 * 1024 * 1024, 1).is_err());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }
}
