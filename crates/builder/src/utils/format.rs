/// File size and formatting utilities
use sps2_errors::{BuildError, Error};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

/// Information about detected compression format
#[derive(Clone, Debug, PartialEq)]
pub struct CompressionFormatInfo {
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

    Ok(CompressionFormatInfo {
        compressed_size: file_size,
    })
}
