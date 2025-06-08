//! Staging directory representation and operations
//!
//! This module provides the StagingDirectory struct that represents
//! a single staging directory and its operations.

use sps2_errors::{Error, InstallError};
use sps2_resolver::PackageId;
use std::path::{Path, PathBuf};
use tokio::fs;
use uuid::Uuid;

/// A staging directory for package extraction and validation
#[derive(Debug)]
pub struct StagingDirectory {
    /// Path to the staging directory
    pub path: PathBuf,
    /// Package ID this staging directory is for
    pub package_id: PackageId,
    /// Unique staging directory ID
    pub staging_id: Uuid,
    /// Whether the extracted content has been validated
    is_validated: bool,
}

impl StagingDirectory {
    /// Create a new staging directory
    #[must_use]
    pub fn new(path: PathBuf, package_id: PackageId, staging_id: Uuid) -> Self {
        Self {
            path,
            package_id,
            staging_id,
            is_validated: false,
        }
    }

    /// Get the path to the staging directory
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Check if the staging directory has been validated
    #[must_use]
    pub fn is_validated(&self) -> bool {
        self.is_validated
    }

    /// Mark the staging directory as validated
    pub fn mark_validated(&mut self) {
        self.is_validated = true;
    }

    /// Get the manifest path
    #[must_use]
    pub fn manifest_path(&self) -> PathBuf {
        self.path.join("manifest.toml")
    }

    /// Get the files directory path
    #[must_use]
    pub fn files_path(&self) -> PathBuf {
        // Check for new package structure first (<package-name>/)
        let new_style_path = self.path.join(&self.package_id.name);

        if new_style_path.exists() {
            new_style_path
        } else {
            // Fall back to old structure (files/)
            self.path.join("files")
        }
    }

    /// Move staging content to final destination atomically
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Content has not been validated
    /// - Atomic move fails
    /// - Destination preparation fails
    pub async fn commit_to_destination(&self, dest_path: &Path) -> Result<(), Error> {
        if !self.is_validated {
            return Err(InstallError::AtomicOperationFailed {
                message: "cannot commit unvalidated staging directory".to_string(),
            }
            .into());
        }

        // Create destination parent directory
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "create_dest_parent".to_string(),
                    path: parent.display().to_string(),
                    message: e.to_string(),
                })?;
        }

        // Atomic move from staging to destination
        fs::rename(&self.path, dest_path).await.map_err(|e| {
            InstallError::AtomicOperationFailed {
                message: format!("failed to move staging to destination: {e}"),
            }
        })?;

        Ok(())
    }

    /// Clean up the staging directory
    ///
    /// # Errors
    ///
    /// Returns an error if directory removal fails
    pub async fn cleanup(&self) -> Result<(), Error> {
        if self.path.exists() {
            fs::remove_dir_all(&self.path)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "cleanup_staging".to_string(),
                    path: self.path.display().to_string(),
                    message: e.to_string(),
                })?;
        }
        Ok(())
    }
}
