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
    /// Speed: 50-100% faster than maximum compression\
    /// Use case: Default for automated builds
    Balanced,

    /// Maximum compression (levels 19-22): Best compression, slower builds
    /// Best for: Production releases, distribution packages
    /// Speed: Baseline (slowest but smallest)
    /// Use case: Final release builds, `sps2 build --max`
    Maximum,

    /// Custom numeric level (1-22): Direct control over zstd compression level
    /// For advanced users who want specific control
    /// Use case: `sps2 build --compression-level 15`
    Custom(u8),
}

impl Default for CompressionLevel {
    fn default() -> Self {
        // Default to balanced for good compromise between speed and size
        Self::Balanced
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
            Self::Balanced => "Balanced compression - Good speed/size tradeoff (default)",
            Self::Maximum => "Maximum compression - Best compression, slower builds",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_level_zstd_levels() {
        assert_eq!(CompressionLevel::Fast.zstd_level(), 3);
        assert_eq!(CompressionLevel::Balanced.zstd_level(), 9);
        assert_eq!(CompressionLevel::Maximum.zstd_level(), 19);
        assert_eq!(CompressionLevel::Custom(1).zstd_level(), 1);
        assert_eq!(CompressionLevel::Custom(22).zstd_level(), 22);

        // Test clamping
        assert_eq!(CompressionLevel::Custom(0).zstd_level(), 1);
        assert_eq!(CompressionLevel::Custom(25).zstd_level(), 22);
    }

    #[test]
    fn test_compression_level_parsing() {
        assert_eq!(
            CompressionLevel::parse("fast").unwrap(),
            CompressionLevel::Fast
        );
        assert_eq!(
            CompressionLevel::parse("FAST").unwrap(),
            CompressionLevel::Fast
        );
        assert_eq!(
            CompressionLevel::parse("balanced").unwrap(),
            CompressionLevel::Balanced
        );
        assert_eq!(
            CompressionLevel::parse("maximum").unwrap(),
            CompressionLevel::Maximum
        );
        assert_eq!(
            CompressionLevel::parse("max").unwrap(),
            CompressionLevel::Maximum
        );

        assert_eq!(
            CompressionLevel::parse("1").unwrap(),
            CompressionLevel::Custom(1)
        );
        assert_eq!(
            CompressionLevel::parse("15").unwrap(),
            CompressionLevel::Custom(15)
        );
        assert_eq!(
            CompressionLevel::parse("22").unwrap(),
            CompressionLevel::Custom(22)
        );

        // Test invalid inputs
        assert!(CompressionLevel::parse("invalid").is_err());
        assert!(CompressionLevel::parse("0").is_err());
        assert!(CompressionLevel::parse("23").is_err());
        assert!(CompressionLevel::parse("not_a_number").is_err());
    }

    #[test]
    fn test_compression_level_properties() {
        // Test speed multipliers are reasonable
        assert!(
            CompressionLevel::Fast.speed_multiplier()
                > CompressionLevel::Balanced.speed_multiplier()
        );
        assert!(
            CompressionLevel::Balanced.speed_multiplier()
                > CompressionLevel::Maximum.speed_multiplier()
        );
        assert!((CompressionLevel::Maximum.speed_multiplier() - 1.0).abs() < f32::EPSILON);

        // Test size increases are reasonable
        assert!(
            CompressionLevel::Fast.size_increase_percent()
                > CompressionLevel::Balanced.size_increase_percent()
        );
        assert!(
            CompressionLevel::Balanced.size_increase_percent()
                > CompressionLevel::Maximum.size_increase_percent()
        );
        assert!((CompressionLevel::Maximum.size_increase_percent() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compression_config_presets() {
        let fast = CompressionConfig::fast();
        assert_eq!(fast.level, CompressionLevel::Fast);

        let balanced = CompressionConfig::balanced();
        assert_eq!(balanced.level, CompressionLevel::Balanced);

        let maximum = CompressionConfig::maximum();
        assert_eq!(maximum.level, CompressionLevel::Maximum);
    }

    #[test]
    fn test_compression_config_custom() {
        let custom_low = CompressionConfig::custom(5);
        assert_eq!(custom_low.level, CompressionLevel::Custom(5));

        let custom_high = CompressionConfig::custom(18);
        assert_eq!(custom_high.level, CompressionLevel::Custom(18));
    }

    #[test]
    fn test_compression_config_estimates() {
        let fast = CompressionConfig::fast();
        let maximum = CompressionConfig::maximum();

        // Fast should be faster to build
        assert!(fast.build_time_multiplier() > maximum.build_time_multiplier());

        // Fast should produce larger files
        assert!(fast.size_increase_percent() > maximum.size_increase_percent());

        // Summary should contain useful information
        let summary = fast.summary();
        assert!(summary.contains("faster"));
        assert!(summary.contains("size"));
    }

    #[test]
    fn test_compression_level_to_string() {
        assert_eq!(CompressionLevel::Fast.to_string(), "fast");
        assert_eq!(CompressionLevel::Balanced.to_string(), "balanced");
        assert_eq!(CompressionLevel::Maximum.to_string(), "maximum");
        assert_eq!(CompressionLevel::Custom(15).to_string(), "15");
    }

    #[test]
    fn test_serde_serialization() {
        // Test that compression levels can be serialized/deserialized
        let level = CompressionLevel::Balanced;
        let serialized = serde_json::to_string(&level).unwrap();
        let deserialized: CompressionLevel = serde_json::from_str(&serialized).unwrap();
        assert_eq!(level, deserialized);

        let config = CompressionConfig::fast();
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: CompressionConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }
}
