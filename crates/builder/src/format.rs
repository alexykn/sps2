//! Compression format detection utilities for .sp packages

use sps2_errors::{BuildError, Error};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::{CompressionConfig, CompressionLevel};

/// Information about detected compression format
#[derive(Clone, Debug, PartialEq)]
pub struct CompressionFormatInfo {
    /// Detected compression configuration
    pub config: CompressionConfig,
    /// Estimated total compressed size
    pub compressed_size: u64,
}

/// zstd magic number (4 bytes): 0x28B52FFD
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Detect the compression format of a .sp package file
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be opened or read
/// - The file is not a valid zstd-compressed package
/// - I/O operations fail during scanning
pub async fn detect_compression_format(file_path: &Path) -> Result<CompressionFormatInfo, Error> {
    let mut file = File::open(file_path).await?;
    let file_size = file.metadata().await?.len();

    // Read the first 4 bytes to verify this is a zstd file
    let mut magic_bytes = [0u8; 4];
    file.read_exact(&mut magic_bytes).await?;

    if magic_bytes != ZSTD_MAGIC {
        return Err(BuildError::Failed {
            message: format!(
                "Invalid package format: expected zstd magic bytes, got {magic_bytes:?}"
            ),
        }
        .into());
    }

    // All packages use standard zstd compression
    let config = CompressionConfig {
        level: CompressionLevel::Balanced, // Can't determine level from file
    };

    Ok(CompressionFormatInfo {
        config,
        compressed_size: file_size,
    })
}

/// Check if a file has the zstd magic number
#[allow(dead_code)]
pub async fn is_zstd_compressed(file_path: &Path) -> Result<bool, Error> {
    let mut file = File::open(file_path).await?;
    let mut magic_bytes = [0u8; 4];

    match file.read_exact(&mut magic_bytes).await {
        Ok(_) => Ok(magic_bytes == ZSTD_MAGIC),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs;

    #[tokio::test]
    async fn test_detect_non_zstd_file() {
        let temp = tempdir().unwrap();
        let test_file = temp.path().join("not_zstd.dat");

        // Write non-zstd data
        fs::write(&test_file, b"Hello, World!").await.unwrap();

        let result = detect_compression_format(&test_file).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_is_zstd_compressed_false() {
        let temp = tempdir().unwrap();
        let test_file = temp.path().join("not_zstd.dat");

        // Write non-zstd data
        fs::write(&test_file, b"Hello, World!").await.unwrap();

        let result = is_zstd_compressed(&test_file).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_is_zstd_compressed_short_file() {
        let temp = tempdir().unwrap();
        let test_file = temp.path().join("short.dat");

        // Write file shorter than magic number
        fs::write(&test_file, b"Hi").await.unwrap();

        let result = is_zstd_compressed(&test_file).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_is_zstd_compressed_true() {
        let temp = tempdir().unwrap();
        let test_file = temp.path().join("zstd_file.dat");

        // Create a real zstd-compressed file
        let test_data = b"This is test data for compression!".repeat(100);
        let compressed_data = zstd::encode_all(&test_data[..], 1).unwrap();
        fs::write(&test_file, compressed_data).await.unwrap();

        let result = is_zstd_compressed(&test_file).await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_detect_zstd_format() {
        let temp = tempdir().unwrap();
        let test_file = temp.path().join("test.sp");

        // Create a zstd-compressed file
        let test_data = b"Test compression data".repeat(100);
        let compressed_data = zstd::encode_all(&test_data[..], 19).unwrap();
        fs::write(&test_file, compressed_data).await.unwrap();

        let info = detect_compression_format(&test_file).await.unwrap();
        assert_eq!(info.config.level, CompressionLevel::Balanced);
        assert!(info.compressed_size > 0);
    }
}
