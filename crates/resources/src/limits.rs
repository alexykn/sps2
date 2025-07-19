//! Resource limit configuration and availability tracking
//!
//! This module defines resource limits and provides utilities for tracking
//! resource availability across concurrent operations.

use serde::{Deserialize, Serialize};

/// Resource limit configuration
///
/// This structure holds configuration for various resource limits
/// used throughout the installation process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum number of concurrent downloads
    pub concurrent_downloads: usize,
    /// Maximum number of concurrent decompressions
    pub concurrent_decompressions: usize,
    /// Maximum number of concurrent package installations
    pub concurrent_installations: usize,
    /// Maximum memory usage in bytes (None = unlimited)
    pub memory_usage: Option<u64>,
}

impl ResourceLimits {
    /// Create default resource limits
    pub fn default() -> Self {
        Self {
            concurrent_downloads: 4,
            concurrent_decompressions: 2,
            concurrent_installations: 1,
            memory_usage: None,
        }
    }

    /// Create resource limits for testing (lower limits)
    pub fn for_testing() -> Self {
        Self {
            concurrent_downloads: 2,
            concurrent_decompressions: 1,
            concurrent_installations: 1,
            memory_usage: Some(100 * 1024 * 1024), // 100MB
        }
    }

    /// Create resource limits based on system capabilities
    pub fn from_system() -> Self {
        let cpu_count = std::thread::available_parallelism()
            .map(std::num::NonZero::get)
            .unwrap_or(4);

        Self {
            concurrent_downloads: cpu_count.min(8),
            concurrent_decompressions: (cpu_count / 2).max(1),
            concurrent_installations: 1, // Keep installations sequential for safety
            memory_usage: None,
        }
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self::default()
    }
}

/// Trait for converting pipeline configurations to resource limits
///
/// This trait allows different pipeline configuration types to be converted
/// into ResourceLimits for use with the ResourceManager.
pub trait IntoResourceLimits {
    /// Convert this configuration into ResourceLimits
    fn into_resource_limits(self) -> ResourceLimits;
}

/// Resource availability information
#[derive(Debug, Clone)]
pub struct ResourceAvailability {
    /// Number of download permits currently available
    pub download: usize,
    /// Number of decompression permits currently available
    pub decompression: usize,
    /// Number of installation permits currently available
    pub installation: usize,
}

impl ResourceAvailability {
    /// Check if any resources are available
    pub fn has_any_available(&self) -> bool {
        self.download > 0 || self.decompression > 0 || self.installation > 0
    }

    /// Check if all resources are fully available
    pub fn all_available(&self, limits: &ResourceLimits) -> bool {
        self.download >= limits.concurrent_downloads
            && self.decompression >= limits.concurrent_decompressions
            && self.installation >= limits.concurrent_installations
    }
}
