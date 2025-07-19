//! Staging directory manager for secure package extraction
//!
//! This module provides the main StagingManager struct that coordinates
//! staging directory creation, validation, and cleanup operations.

use crate::{validate_sp_file, ValidationResult};
use sps2_errors::{Error, InstallError};
use sps2_events::{Event, EventEmitter};
use sps2_resolver::PackageId;
use sps2_resources::ResourceManager;
use sps2_store::{extract_package_with_events, PackageStore};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// Staging directory manager for secure package extraction
pub struct StagingManager {
    /// Base path for staging directories
    base_path: PathBuf,
    /// Package store for extraction operations
    #[allow(dead_code)]
    store: PackageStore,
    /// Resource manager for concurrency control
    resources: Arc<ResourceManager>,
}

impl StagingManager {
    /// Create a new staging manager
    ///
    /// # Errors
    ///
    /// Returns an error if the staging base directory cannot be created
    pub async fn new(
        store: PackageStore,
        base_staging_path: PathBuf,
        resources: Arc<ResourceManager>,
    ) -> Result<Self, Error> {
        let base_path = base_staging_path;

        // Create staging base directory if it doesn't exist
        fs::create_dir_all(&base_path)
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "create_staging_base".to_string(),
                path: base_path.display().to_string(),
                message: e.to_string(),
            })?;

        Ok(Self {
            base_path,
            store,
            resources,
        })
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
        let _permit = self.resources.acquire_installation_permit().await?;

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
    pub async fn extract_to_staging<T: EventEmitter>(
        &self,
        sp_file: &Path,
        package_id: &PackageId,
        context: &T,
    ) -> Result<StagingDirectory, Error> {
        self.extract_to_staging_internal(sp_file, package_id, context, true)
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
    pub async fn extract_validated_tar_to_staging<T: EventEmitter>(
        &self,
        tar_file: &Path,
        package_id: &PackageId,
        context: &T,
    ) -> Result<StagingDirectory, Error> {
        self.extract_to_staging_internal(tar_file, package_id, context, false)
            .await
    }

    /// Internal method for extraction with optional validation
    async fn extract_to_staging_internal<T: EventEmitter>(
        &self,
        file_path: &Path,
        package_id: &PackageId,
        context: &T,
        validate_as_sp: bool,
    ) -> Result<StagingDirectory, Error> {
        context.emit_event(Event::OperationStarted {
            operation: format!("Creating staging directory for {}", package_id.name),
        });

        // Validate the file if required (only for .sp files, not pre-validated tar files)
        let validation_result = if validate_as_sp {
            let result = validate_sp_file(file_path, context.event_sender()).await?;
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

        context.emit_event(Event::OperationStarted {
            operation: format!(
                "Extracting package to staging: {}",
                staging_dir.path.display()
            ),
        });

        // Extract package to staging directory
        extract_package_with_events(file_path, &staging_dir.path, context.event_sender())
            .await
            .map_err(|e| InstallError::ExtractionFailed {
                message: format!("failed to extract to staging: {e}"),
            })?;

        context.emit_debug(format!(
            "DEBUG: Extraction completed, listing directory: {}",
            staging_dir.path.display()
        ));
        if let Ok(mut entries) = tokio::fs::read_dir(&staging_dir.path).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let file_name = entry.file_name().to_string_lossy().to_string();
                let file_type = if entry.file_type().await.is_ok_and(|ft| ft.is_dir()) {
                    "directory"
                } else {
                    "file"
                };
                context.emit_debug(format!("DEBUG: Extracted {file_type}: {file_name}"));
            }
        }

        // Post-extraction validation (update file count for tar files)
        let mut adjusted_validation_result = validation_result;
        if !validate_as_sp {
            // For tar files, get actual file count after extraction
            adjusted_validation_result.file_count = count_files(&staging_dir.path).await?;
        }

        self.validate_extracted_content(&mut staging_dir, &adjusted_validation_result, context)
            .await?;

        context.emit_event(Event::OperationCompleted {
            operation: format!("Package staged successfully: {}", package_id.name),
            success: true,
        });

        Ok(staging_dir)
    }

    /// Validate extracted content in staging directory
    async fn validate_extracted_content<T: EventEmitter>(
        &self,
        staging_dir: &mut StagingDirectory,
        validation_result: &ValidationResult,
        context: &T,
    ) -> Result<(), Error> {
        context.emit_event(Event::OperationStarted {
            operation: "Validating extracted content".to_string(),
        });
        context.emit_debug(format!(
            "DEBUG: Starting post-extraction validation for: {}",
            staging_dir.path.display()
        ));

        // Step 1: Verify manifest exists and is valid
        let manifest = verify_and_parse_manifest(staging_dir, context.event_sender()).await?;

        // Step 2: Verify package identity matches expected
        verify_package_identity(staging_dir, &manifest, context.event_sender())?;

        // Step 3: Verify file count consistency
        verify_file_count(staging_dir, validation_result, context.event_sender()).await?;

        // Step 4: Verify directory structure is safe
        verify_directory_structure(staging_dir, context.event_sender()).await?;

        // Mark as validated
        staging_dir.mark_validated();

        context.emit_debug("DEBUG: Marked staging directory as validated".to_string());
        context.emit_event(Event::OperationCompleted {
            operation: "Content validation completed".to_string(),
            success: true,
        });

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
