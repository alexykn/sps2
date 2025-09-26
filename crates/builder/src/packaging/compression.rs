//! Zstandard compression for sps2 packages
//!
//! This module applies a fixed Zstandard compression level when creating
//! package archives, ensuring consistent output across builds.

use sps2_errors::Error;
use std::path::Path;

const DEFAULT_LEVEL: i32 = 9;

/// Compress tar archive with zstd using async-compression
/// Compress a tar file using Zstandard compression
///
/// # Errors
///
/// Returns an error if file I/O operations fail or compression fails.
pub async fn compress_with_zstd(tar_path: &Path, output_path: &Path) -> Result<(), Error> {
    use async_compression::tokio::write::ZstdEncoder;
    use async_compression::Level;
    use tokio::fs::File;
    use tokio::io::{AsyncWriteExt, BufReader};

    let input_file = File::open(tar_path).await?;
    let output_file = File::create(output_path).await?;

    // Create zstd encoder with default compression level
    let level = Level::Precise(DEFAULT_LEVEL);
    let mut encoder = ZstdEncoder::with_quality(output_file, level);

    // Copy tar file through zstd encoder
    let mut reader = BufReader::new(input_file);
    tokio::io::copy(&mut reader, &mut encoder).await?;

    // Ensure all data is written
    encoder.shutdown().await?;

    Ok(())
}
