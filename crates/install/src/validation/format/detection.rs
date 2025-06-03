//! Package format detection functionality
//!
//! This module handles detection of package formats by reading magic bytes
//! and examining file headers to determine if a file is a zstd-compressed
//! tar archive or a plain tar archive.

use sps2_errors::{Error, InstallError};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};

use crate::validation::types::{PackageFormat, ZSTD_MAGIC};

/// Detects package format by reading magic bytes
///
/// This function reads the first few bytes of a file to determine if it's
/// a zstd-compressed archive or a plain tar archive. It handles the detection
/// robustly with proper error handling.
///
/// # Errors
///
/// Returns an error if:
/// - File cannot be opened or read
/// - File is too small to determine format
/// - Format is unrecognized (not zstd or tar)
pub async fn detect_package_format(file_path: &Path) -> Result<PackageFormat, Error> {
    let file = File::open(file_path)
        .await
        .map_err(|e| InstallError::InvalidPackageFile {
            path: file_path.display().to_string(),
            message: format!("failed to open file: {e}"),
        })?;

    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 4];

    // Read first 4 bytes to check for zstd magic number
    let bytes_read =
        reader
            .read(&mut magic)
            .await
            .map_err(|e| InstallError::InvalidPackageFile {
                path: file_path.display().to_string(),
                message: format!("failed to read magic bytes: {e}"),
            })?;

    if bytes_read < 4 {
        return Err(InstallError::InvalidPackageFile {
            path: file_path.display().to_string(),
            message: "file too small to determine format".to_string(),
        }
        .into());
    }

    if magic == ZSTD_MAGIC {
        return Ok(PackageFormat::ZstdCompressed);
    }

    // Check for tar header at the beginning
    // Seek back to start and read tar header
    drop(reader); // Close the reader
    let file = File::open(file_path)
        .await
        .map_err(|e| InstallError::InvalidPackageFile {
            path: file_path.display().to_string(),
            message: format!("failed to reopen file: {e}"),
        })?;

    let mut reader = BufReader::new(file);
    let mut header = [0u8; 512]; // tar header is 512 bytes

    let bytes_read =
        reader
            .read(&mut header)
            .await
            .map_err(|e| InstallError::InvalidPackageFile {
                path: file_path.display().to_string(),
                message: format!("failed to read tar header: {e}"),
            })?;

    if bytes_read < 512 {
        return Ok(PackageFormat::Unknown);
    }

    // Check for "ustar" magic at offset 257
    if &header[257..262] == b"ustar" {
        return Ok(PackageFormat::PlainTar);
    }

    Err(InstallError::InvalidPackageFile {
        path: file_path.display().to_string(),
        message: "unrecognized file format (not zstd or tar)".to_string(),
    }
    .into())
}

/// Validates that the detected format is supported
///
/// This function checks that the detected format is one that the system
/// can handle for package installation.
pub fn validate_supported_format(format: &PackageFormat) -> Result<(), Error> {
    match format {
        PackageFormat::ZstdCompressed | PackageFormat::PlainTar => Ok(()),
        PackageFormat::Unknown => Err(InstallError::InvalidPackageFile {
            path: "unknown".to_string(),
            message: "unknown package format is not supported".to_string(),
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_detect_zstd_format() {
        let mut temp_file = NamedTempFile::with_suffix(".sp").unwrap();
        // Write zstd magic bytes
        temp_file.write_all(&ZSTD_MAGIC).unwrap();
        temp_file.write_all(b"more content").unwrap();

        let result = detect_package_format(temp_file.path()).await;
        assert!(result.is_ok());

        if let Ok(format) = result {
            assert_eq!(format, PackageFormat::ZstdCompressed);
        }
    }

    #[tokio::test]
    async fn test_detect_unknown_format() {
        let mut temp_file = NamedTempFile::with_suffix(".sp").unwrap();
        // Write enough content to get past the size check but not a valid tar header
        temp_file.write_all(&[0u8; 1024]).unwrap(); // 1024 zero bytes

        let result = detect_package_format(temp_file.path()).await;
        assert!(result.is_err());

        if let Err(e) = result {
            let error_message = e.to_string();
            assert!(error_message.contains("unrecognized file format"));
        }
    }

    #[tokio::test]
    async fn test_file_too_small() {
        let mut temp_file = NamedTempFile::with_suffix(".sp").unwrap();
        temp_file.write_all(b"ab").unwrap(); // Only 2 bytes

        let result = detect_package_format(temp_file.path()).await;
        assert!(result.is_err());

        if let Err(e) = result {
            let error_message = e.to_string();
            assert!(error_message.contains("file too small to determine format"));
        }
    }

    #[test]
    fn test_validate_supported_format() {
        assert!(validate_supported_format(&PackageFormat::ZstdCompressed).is_ok());
        assert!(validate_supported_format(&PackageFormat::PlainTar).is_ok());
        assert!(validate_supported_format(&PackageFormat::Unknown).is_err());
    }
}
