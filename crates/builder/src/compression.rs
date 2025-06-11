//! Compression level management and ZSTD compression for sps2 packages
//!
//! This module provides configurable compression levels to optimize the tradeoff
//! between build speed and package size for different use cases:
//! - Development: Fast builds for iteration (larger files acceptable)
//! - CI/CD: Balanced performance for build pipelines
//! - Production: Maximum compression for distribution

use serde::{Deserialize, Serialize};
use sps2_errors::Error;
use std::path::Path;

/// Compression levels with clear speed vs size tradeoffs
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionLevel {
    /// Fast compression (levels 1-3): Quick builds, larger files (~20% larger)
    /// Best for: Development builds, iteration, testing
    /// Speed: 2-5x faster than maximum compression
    /// Use case: `sps2 build --fast` or during development
    Fast,

    /// Balanced compression (levels 6-9): Good compression, reasonable speed
    /// Best for: CI/CD builds, automated pipelines
    /// Speed: 50-100% faster than maximum compression
    /// Use case: Automated builds where speed matters
    Balanced,

    /// Maximum compression (levels 19-22): Best compression, slower builds
    /// Best for: Production releases, distribution packages
    /// Speed: Baseline (slowest but smallest)
    /// Use case: Default for all builds, `sps2 build --max`
    Maximum,

    /// Custom numeric level (1-22): Direct control over zstd compression level
    /// For advanced users who want specific control
    /// Use case: `sps2 build --compression-level 15`
    Custom(u8),
}

impl Default for CompressionLevel {
    fn default() -> Self {
        // Default to maximum compression for smallest package sizes
        Self::Maximum
    }
}

impl std::fmt::Display for CompressionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fast => write!(f, "fast"),
            Self::Balanced => write!(f, "balanced"),
            Self::Maximum => write!(f, "maximum"),
            Self::Custom(level) => write!(f, "{level}"),
        }
    }
}

impl CompressionLevel {
    /// Get the actual zstd compression level for this setting
    #[must_use]
    pub fn zstd_level(&self) -> i32 {
        match self {
            Self::Fast => 3,     // Fast but still reasonable compression
            Self::Balanced => 9, // Good balance of speed vs compression
            Self::Maximum => 19, // Maximum compression (zstd default max)
            Self::Custom(level) => {
                // Clamp to valid zstd range (1-22)
                i32::from(*level).clamp(1, 22)
            }
        }
    }

    /// Get a human-readable description of this compression level
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Fast => "Fast compression - Quick builds, larger files (2-5x faster)",
            Self::Balanced => "Balanced compression - Good speed/size tradeoff",
            Self::Maximum => "Maximum compression - Best compression, slower builds (default)",
            Self::Custom(level) => match *level {
                1..=3 => "Custom fast compression",
                4..=9 => "Custom balanced compression",
                10..=22 => "Custom high compression",
                _ => "Custom compression level",
            },
        }
    }

    /// Get estimated speed multiplier compared to maximum compression
    #[must_use]
    pub fn speed_multiplier(&self) -> f32 {
        match self {
            Self::Fast => 4.0,     // ~4x faster than maximum
            Self::Balanced => 2.0, // ~2x faster than maximum
            Self::Maximum => 1.0,  // Baseline
            Self::Custom(level) => {
                // Rough estimate based on zstd level
                match *level {
                    1..=3 => 4.0,
                    4..=6 => 3.0,
                    7..=9 => 2.0,
                    10..=15 => 1.5,
                    _ => 1.0,
                }
            }
        }
    }

    /// Get estimated size increase compared to maximum compression
    #[must_use]
    pub fn size_increase_percent(&self) -> f32 {
        match self {
            Self::Fast => 20.0,     // ~20% larger than maximum
            Self::Balanced => 10.0, // ~10% larger than maximum
            Self::Maximum => 0.0,   // Baseline (smallest)
            Self::Custom(level) => {
                // Rough estimate based on zstd level
                match *level {
                    1..=3 => 20.0,
                    4..=6 => 15.0,
                    7..=9 => 10.0,
                    10..=15 => 5.0,
                    _ => 0.0,
                }
            }
        }
    }

    /// Parse compression level from string (for CLI and config)
    ///
    /// # Errors
    ///
    /// Returns an error if the string cannot be parsed as a valid compression level
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "fast" => Ok(Self::Fast),
            "balanced" => Ok(Self::Balanced),
            "maximum" | "max" => Ok(Self::Maximum),
            _ => {
                // Try to parse as a number
                if let Ok(level) = s.parse::<u8>() {
                    if (1..=22).contains(&level) {
                        Ok(Self::Custom(level))
                    } else {
                        Err(format!(
                            "Compression level must be between 1 and 22, got {level}"
                        ))
                    }
                } else {
                    Err(format!(
                        "Invalid compression level '{s}'. Valid options: fast, balanced, maximum, or 1-22"
                    ))
                }
            }
        }
    }
}

/// Compression configuration for zstd compression
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompressionConfig {
    /// Compression level to use
    pub level: CompressionLevel,
}

impl CompressionConfig {
    /// Create config for fast builds
    #[must_use]
    pub fn fast() -> Self {
        Self {
            level: CompressionLevel::Fast,
        }
    }

    /// Create config for balanced builds
    #[must_use]
    pub fn balanced() -> Self {
        Self {
            level: CompressionLevel::Balanced,
        }
    }

    /// Create config for maximum compression
    #[must_use]
    pub fn maximum() -> Self {
        Self {
            level: CompressionLevel::Maximum,
        }
    }

    /// Create config with custom level
    #[must_use]
    pub fn custom(level: u8) -> Self {
        Self {
            level: CompressionLevel::Custom(level),
        }
    }

    /// Get estimated build time multiplier
    #[must_use]
    pub fn build_time_multiplier(&self) -> f32 {
        self.level.speed_multiplier()
    }

    /// Get estimated package size increase percentage
    #[must_use]
    pub fn size_increase_percent(&self) -> f32 {
        self.level.size_increase_percent()
    }

    /// Get a summary of the compression configuration for display
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} ({:.1}x faster, +{:.1}% size)",
            self.level.description(),
            self.build_time_multiplier(),
            self.size_increase_percent()
        )
    }
}

/// Compress tar archive with zstd using async-compression
/// Compress a tar file using Zstandard compression
///
/// # Errors
///
/// Returns an error if file I/O operations fail or compression fails.
pub async fn compress_with_zstd(
    config: &CompressionConfig,
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
    let compression_level = config.level.zstd_level();
    let level = Level::Precise(compression_level);
    let mut encoder = ZstdEncoder::with_quality(output_file, level);

    // Copy tar file through zstd encoder
    let mut reader = BufReader::new(input_file);
    tokio::io::copy(&mut reader, &mut encoder).await?;

    // Ensure all data is written
    encoder.shutdown().await?;

    Ok(())
}
