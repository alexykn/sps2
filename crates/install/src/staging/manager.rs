//! Staging directory manager for secure package extraction
//!
//! This module provides the main StagingManager struct that coordinates
//! staging directory creation, validation, and cleanup operations.

use crate::{validate_sp_file, ValidationResult};
use sps2_errors::{Error, InstallError};
use sps2_events::{Event, EventSender};
use sps2_resolver::PackageId;
use sps2_store::{extract_package_with_events, PackageStore};
use std::path::{Path, PathBuf};
use tokio::fs;
use uuid::Uuid;

use super::{
    directory::StagingDirectory,
    utils::count_files,
    validation::{
        verify_and_parse_manifest, verify_directory_structure, verify_file_count,
        verify_package_identity,
    },
};

/// Maximum number of staging directories to allow simultaneously
const MAX_STAGING_DIRS: usize = 100;

/// Staging directory manager for secure package extraction
pub struct StagingManager {
    /// Base path for staging directories
    base_path: PathBuf,
    /// Package store for extraction operations
    #[allow(dead_code)]
    store: PackageStore,
}

impl StagingManager {
    /// Create a new staging manager
    ///
    /// # Errors
    ///
    /// Returns an error if the staging base directory cannot be created
    pub async fn new(store: PackageStore, base_staging_path: PathBuf) -> Result<Self, Error> {
        let base_path = base_staging_path;

        // Create staging base directory if it doesn't exist
        fs::create_dir_all(&base_path)
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "create_staging_base".to_string(),
                path: base_path.display().to_string(),
                message: e.to_string(),
            })?;

        Ok(Self { base_path, store })
    }

    /// Create a new staging directory for a package
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Too many staging directories exist
    /// - Directory creation fails
    /// - Permissions cannot be set
    pub async fn create_staging_dir(
        &self,
        package_id: &PackageId,
    ) -> Result<StagingDirectory, Error> {
        // Check staging directory count limit
        self.check_staging_limit().await?;

        // Generate unique staging directory name
        let staging_id = Uuid::new_v4();
        let dir_name = format!("{}-{}-{staging_id}", package_id.name, package_id.version);
        let staging_path = self.base_path.join(dir_name);

        // Create the staging directory with secure permissions
        fs::create_dir_all(&staging_path)
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "create_staging_dir".to_string(),
                path: staging_path.display().to_string(),
                message: e.to_string(),
            })?;

        // Set restrictive permissions on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(0o700); // Owner read/write/execute only
            fs::set_permissions(&staging_path, permissions)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "set_staging_permissions".to_string(),
                    path: staging_path.display().to_string(),
                    message: e.to_string(),
                })?;
        }

        Ok(StagingDirectory::new(
            staging_path,
            package_id.clone(),
            staging_id,
        ))
    }

    /// Extract and validate a package to a staging directory
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Package validation fails
    /// - Extraction fails
    /// - Post-extraction validation fails
    pub async fn extract_to_staging(
        &self,
        sp_file: &Path,
        package_id: &PackageId,
        event_sender: Option<&EventSender>,
    ) -> Result<StagingDirectory, Error> {
        self.extract_to_staging_internal(sp_file, package_id, event_sender, true)
            .await
    }

    /// Extract and validate a pre-validated tar file to a staging directory
    /// This skips .sp file validation since the content has already been validated
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Extraction fails
    /// - Post-extraction validation fails
    pub async fn extract_validated_tar_to_staging(
        &self,
        tar_file: &Path,
        package_id: &PackageId,
        event_sender: Option<&EventSender>,
    ) -> Result<StagingDirectory, Error> {
        self.extract_to_staging_internal(tar_file, package_id, event_sender, false)
            .await
    }

    /// Internal method for extraction with optional validation
    async fn extract_to_staging_internal(
        &self,
        file_path: &Path,
        package_id: &PackageId,
        event_sender: Option<&EventSender>,
        validate_as_sp: bool,
    ) -> Result<StagingDirectory, Error> {
        // Send staging started event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::OperationStarted {
                operation: format!("Creating staging directory for {}", package_id.name),
            });
        }

        // Validate the file if required (only for .sp files, not pre-validated tar files)
        let validation_result = if validate_as_sp {
            let result = validate_sp_file(file_path, event_sender).await?;
            if !result.is_valid {
                return Err(InstallError::InvalidPackageFile {
                    path: file_path.display().to_string(),
                    message: "package validation failed".to_string(),
                }
                .into());
            }
            result
        } else {
            // For pre-validated tar files, create a minimal validation result
            let mut result = ValidationResult::new(crate::PackageFormat::PlainTar);
            result.mark_valid();
            result.file_count = 1; // Will be updated after extraction
            result.extracted_size = 1024; // Will be updated after extraction
            result
        };

        // Create staging directory
        let mut staging_dir = self.create_staging_dir(package_id).await?;

        // Send extraction started event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::OperationStarted {
                operation: format!(
                    "Extracting package to staging: {}",
                    staging_dir.path.display()
                ),
            });
        }

        // Extract package to staging directory
        extract_package_with_events(file_path, &staging_dir.path, event_sender)
            .await
            .map_err(|e| InstallError::ExtractionFailed {
                message: format!("failed to extract to staging: {e}"),
            })?;

        // Debug: List what was actually extracted
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "DEBUG: Extraction completed, listing directory: {}",
                    staging_dir.path.display()
                ),
                context: std::collections::HashMap::new(),
            });
            if let Ok(mut entries) = tokio::fs::read_dir(&staging_dir.path).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let file_name = entry.file_name().to_string_lossy().to_string();
                    let file_type = if entry.file_type().await.is_ok_and(|ft| ft.is_dir()) {
                        "directory"
                    } else {
                        "file"
                    };
                    let _ = sender.send(Event::DebugLog {
                        message: format!("DEBUG: Extracted {file_type}: {file_name}"),
                        context: std::collections::HashMap::new(),
                    });
                }
            }
        }

        // Post-extraction validation (update file count for tar files)
        let mut adjusted_validation_result = validation_result;
        if !validate_as_sp {
            // For tar files, get actual file count after extraction
            adjusted_validation_result.file_count = count_files(&staging_dir.path).await?;
        }

        self.validate_extracted_content(
            &mut staging_dir,
            &adjusted_validation_result,
            event_sender,
        )
        .await?;

        // Send staging completed event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::OperationCompleted {
                operation: format!("Package staged successfully: {}", package_id.name),
                success: true,
            });
        }

        Ok(staging_dir)
    }

    /// Validate extracted content in staging directory
    async fn validate_extracted_content(
        &self,
        staging_dir: &mut StagingDirectory,
        validation_result: &ValidationResult,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::OperationStarted {
                operation: "Validating extracted content".to_string(),
            });
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "DEBUG: Starting post-extraction validation for: {}",
                    staging_dir.path.display()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        // Step 1: Verify manifest exists and is valid
        let manifest = verify_and_parse_manifest(staging_dir, event_sender).await?;

        // Step 2: Verify package identity matches expected
        verify_package_identity(staging_dir, &manifest, event_sender)?;

        // Step 3: Verify file count consistency
        verify_file_count(staging_dir, validation_result, event_sender).await?;

        // Step 4: Verify directory structure is safe
        verify_directory_structure(staging_dir, event_sender).await?;

        // Mark as validated
        staging_dir.mark_validated();

        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: "DEBUG: Marked staging directory as validated".to_string(),
                context: std::collections::HashMap::new(),
            });
            let _ = sender.send(Event::OperationCompleted {
                operation: "Content validation completed".to_string(),
                success: true,
            });
        }

        Ok(())
    }

    /// Check if too many staging directories exist
    async fn check_staging_limit(&self) -> Result<(), Error> {
        let mut count = 0;
        let mut entries =
            fs::read_dir(&self.base_path)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "read_staging_base".to_string(),
                    path: self.base_path.display().to_string(),
                    message: e.to_string(),
                })?;

        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "count_staging_dirs".to_string(),
                    path: self.base_path.display().to_string(),
                    message: e.to_string(),
                })?
        {
            if entry
                .file_type()
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "check_staging_file_type".to_string(),
                    path: entry.path().display().to_string(),
                    message: e.to_string(),
                })?
                .is_dir()
            {
                count += 1;
                if count > MAX_STAGING_DIRS {
                    return Err(InstallError::ConcurrencyError {
                        message: format!(
                            "too many staging directories: {count} (max: {MAX_STAGING_DIRS})"
                        ),
                    }
                    .into());
                }
            }
        }

        Ok(())
    }

    /// Clean up old staging directories
    ///
    /// # Errors
    ///
    /// Returns an error if directory cleanup fails
    pub async fn cleanup_old_staging_dirs(&self) -> Result<usize, Error> {
        let mut cleaned = 0;
        let mut entries =
            fs::read_dir(&self.base_path)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "cleanup_staging_dirs".to_string(),
                    path: self.base_path.display().to_string(),
                    message: e.to_string(),
                })?;

        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "cleanup_staging_entry".to_string(),
                    path: self.base_path.display().to_string(),
                    message: e.to_string(),
                })?
        {
            let entry_path = entry.path();

            if entry
                .file_type()
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "cleanup_staging_file_type".to_string(),
                    path: entry_path.display().to_string(),
                    message: e.to_string(),
                })?
                .is_dir()
            {
                // Check if directory is old (more than 1 hour)
                if let Ok(metadata) = entry.metadata().await {
                    if let Ok(created) = metadata.created() {
                        if let Ok(elapsed) = created.elapsed() {
                            if elapsed.as_secs() > 3600 {
                                // 1 hour
                                if fs::remove_dir_all(&entry_path).await.is_ok() {
                                    cleaned += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(cleaned)
    }
}
