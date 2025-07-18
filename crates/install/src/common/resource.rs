//! Resource management utilities for the install crate
//!
//! This module provides helper functions for managing resources like
//! semaphores, memory limits, and concurrency control.

use sps2_errors::{Error, InstallError};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Acquire a semaphore permit with proper error handling
///
/// This helper function provides consistent error handling for semaphore
/// acquisition across all modules in the install crate.
///
/// # Arguments
///
/// * `semaphore` - The semaphore to acquire a permit from
/// * `operation` - Description of the operation for error reporting
///
/// # Errors
///
/// Returns an error if the semaphore is closed or acquisition fails
pub async fn acquire_semaphore_permit(
    semaphore: Arc<Semaphore>,
    operation: &str,
) -> Result<OwnedSemaphorePermit, Error> {
    semaphore.clone().acquire_owned().await.map_err(|_| {
        InstallError::ConcurrencyError {
            message: format!("failed to acquire semaphore for {}", operation),
        }
        .into()
    })
}

/// Try to acquire a semaphore permit without waiting
///
/// This helper function attempts to acquire a semaphore permit immediately
/// without blocking. Useful for checking resource availability.
///
/// # Arguments
///
/// * `semaphore` - The semaphore to try to acquire a permit from
///
/// # Returns
///
/// Returns `Ok(Some(permit))` if successful, `Ok(None)` if would block,
/// or an error if the semaphore is closed.
pub fn try_acquire_semaphore_permit(
    semaphore: &Arc<Semaphore>,
) -> Result<Option<OwnedSemaphorePermit>, Error> {
    match semaphore.clone().try_acquire_owned() {
        Ok(permit) => Ok(Some(permit)),
        Err(tokio::sync::TryAcquireError::NoPermits) => Ok(None),
        Err(tokio::sync::TryAcquireError::Closed) => Err(InstallError::ConcurrencyError {
            message: "semaphore is closed".to_string(),
        }
        .into()),
    }
}

/// Create a semaphore with a specified number of permits
///
/// This is a convenience function for creating semaphores with consistent
/// error handling and documentation.
///
/// # Arguments
///
/// * `permits` - Number of permits the semaphore should have
///
/// # Returns
///
/// Returns an Arc-wrapped semaphore for shared ownership
pub fn create_semaphore(permits: usize) -> Arc<Semaphore> {
    Arc::new(Semaphore::new(permits))
}

/// Resource limit configuration
///
/// This structure holds configuration for various resource limits
/// used throughout the installation process.
#[derive(Debug, Clone)]
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

use crate::pipeline::config::PipelineConfig;

impl From<PipelineConfig> for ResourceLimits {
    fn from(config: PipelineConfig) -> Self {
        Self {
            concurrent_downloads: config.max_downloads,
            concurrent_decompressions: config.max_decompressions,
            concurrent_installations: config.max_validations,
            memory_usage: Some(config.memory_limit),
        }
    }
}

/// Resource manager for coordinating resource usage
///
/// This structure manages semaphores and resource limits for the
/// installation process, ensuring we don't exceed system capabilities.
#[derive(Debug)]
pub struct ResourceManager {
    /// Semaphore for download operations
    pub download_semaphore: Arc<Semaphore>,
    /// Semaphore for decompression operations
    pub decompression_semaphore: Arc<Semaphore>,
    /// Semaphore for installation operations
    pub installation_semaphore: Arc<Semaphore>,
    /// Resource limits configuration
    pub limits: ResourceLimits,
    /// Current memory usage
    pub memory_usage: Arc<AtomicU64>,
}

impl ResourceManager {
    /// Create a new resource manager with the given limits
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            download_semaphore: create_semaphore(limits.concurrent_downloads),
            decompression_semaphore: create_semaphore(limits.concurrent_decompressions),
            installation_semaphore: create_semaphore(limits.concurrent_installations),
            memory_usage: Arc::new(AtomicU64::new(0)),
            limits,
        }
    }

    /// Create a resource manager with default limits
    pub fn default() -> Self {
        Self::new(ResourceLimits::default())
    }

    /// Create a resource manager with system-based limits
    pub fn from_system() -> Self {
        Self::new(ResourceLimits::from_system())
    }

    /// Acquire a download permit
    pub async fn acquire_download_permit(&self) -> Result<OwnedSemaphorePermit, Error> {
        acquire_semaphore_permit(self.download_semaphore.clone(), "download").await
    }

    /// Acquire a decompression permit
    pub async fn acquire_decompression_permit(&self) -> Result<OwnedSemaphorePermit, Error> {
        acquire_semaphore_permit(self.decompression_semaphore.clone(), "decompression").await
    }

    /// Acquire an installation permit
    pub async fn acquire_installation_permit(&self) -> Result<OwnedSemaphorePermit, Error> {
        acquire_semaphore_permit(self.installation_semaphore.clone(), "installation").await
    }

    /// Try to acquire a download permit without blocking
    pub fn try_acquire_download_permit(&self) -> Result<Option<OwnedSemaphorePermit>, Error> {
        try_acquire_semaphore_permit(&self.download_semaphore)
    }

    /// Try to acquire a decompression permit without blocking
    pub fn try_acquire_decompression_permit(&self) -> Result<Option<OwnedSemaphorePermit>, Error> {
        try_acquire_semaphore_permit(&self.decompression_semaphore)
    }

    /// Try to acquire an installation permit without blocking
    pub fn try_acquire_installation_permit(&self) -> Result<Option<OwnedSemaphorePermit>, Error> {
        try_acquire_semaphore_permit(&self.installation_semaphore)
    }

    /// Check if memory usage is within limits
    pub fn is_memory_within_limits(&self, current_usage: u64) -> bool {
        match self.limits.memory_usage {
            Some(limit) => current_usage <= limit,
            None => true, // No limit set
        }
    }

    /// Get current resource availability
    pub fn get_resource_availability(&self) -> ResourceAvailability {
        ResourceAvailability {
            download: self.download_semaphore.available_permits(),
            decompression: self.decompression_semaphore.available_permits(),
            installation: self.installation_semaphore.available_permits(),
        }
    }

    /// Clean up resources
    pub fn cleanup(&self) -> Result<(), Error> {
        // Nothing to do here for now, but this can be used to clean up
        // any temporary files or other resources created by the resource manager.
        Ok(())
    }
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
