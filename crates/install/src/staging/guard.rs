//! RAII guard for automatic staging directory cleanup
//!
//! This module provides the StagingGuard struct that ensures
//! staging directories are cleaned up even if operations fail.

use sps2_errors::{Error, InstallError};
use tokio::fs;

use super::directory::StagingDirectory;

/// RAII guard for automatic staging directory cleanup
#[derive(Debug)]
pub struct StagingGuard {
    staging_dir: Option<StagingDirectory>,
}

impl StagingGuard {
    /// Create a new staging guard
    #[must_use]
    pub fn new(staging_dir: StagingDirectory) -> Self {
        Self {
            staging_dir: Some(staging_dir),
        }
    }

    /// Take ownership of the staging directory, preventing cleanup
    ///
    /// # Errors
    ///
    /// Returns an error if the staging directory was already taken
    pub fn take(&mut self) -> Result<StagingDirectory, Error> {
        self.staging_dir.take().ok_or_else(|| {
            InstallError::AtomicOperationFailed {
                message: "staging directory already taken".to_string(),
            }
            .into()
        })
    }

    /// Get a reference to the staging directory
    #[must_use]
    pub fn staging_dir(&self) -> Option<&StagingDirectory> {
        self.staging_dir.as_ref()
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if let Some(staging_dir) = &self.staging_dir {
            // Best effort cleanup - ignore errors in destructor
            let path = staging_dir.path.clone();
            tokio::spawn(async move {
                let _ = fs::remove_dir_all(&path).await;
            });
        }
    }
}
