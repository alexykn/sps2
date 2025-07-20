//! Compression level management and ZSTD compression for sps2 packages
//!
//! This module provides configurable compression levels to optimize the tradeoff
//! between build speed and package size for different use cases:
//! - Development: Fast builds for iteration (larger files acceptable)
//! - CI/CD: Balanced performance for build pipelines
//! - Production: Maximum compression for distribution

use sps2_errors::Error;
use std::path::Path;

/// Compress tar archive with zstd using async-compression
/// Compress a tar file using Zstandard compression
///
/// # Errors
///
/// Returns an error if file I/O operations fail or compression fails.
pub async fn compress_with_zstd(
    settings: &sps2_config::builder::CompressionSettings,
    tar_path: &Path,
    output_path: &Path,
) -> Result<(), Error> {
    use async_compression::tokio::write::ZstdEncoder;
    use async_compression::Level;
    use tokio::fs::File;
    use tokio::io::{AsyncWriteExt, BufReader};

    let input_file = File::open(tar_path).await?;
    let output_file = File::create(output_path).await?;

    // Create zstd encoder with specified compression level
    let compression_level = settings.zstd_level();
    let level = Level::Precise(compression_level);
    let mut encoder = ZstdEncoder::with_quality(output_file, level);

    // Copy tar file through zstd encoder
    let mut reader = BufReader::new(input_file);
    tokio::io::copy(&mut reader, &mut encoder).await?;

    // Ensure all data is written
    encoder.shutdown().await?;

    Ok(())
}
