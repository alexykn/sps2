//! Staging directory management for secure package extraction
//!
//! This module provides secure staging directory creation, validation, and cleanup
//! for package installation. It ensures that packages are extracted to temporary
//! directories, validated, and then atomically moved to their final location.

use crate::{validate_sp_file, ValidationResult};
use sps2_errors::{Error, InstallError};
use sps2_events::{Event, EventSender};
use sps2_manifest::Manifest;
use sps2_resolver::PackageId;
use sps2_store::{extract_package_with_events, PackageStore};
use std::path::{Path, PathBuf};
use tokio::fs;
use uuid::Uuid;

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

        Ok(StagingDirectory {
            path: staging_path,
            package_id: package_id.clone(),
            staging_id,
            is_validated: false,
        })
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
            adjusted_validation_result.file_count = self.count_files(&staging_dir.path).await?;
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
        let manifest = self
            .verify_and_parse_manifest(staging_dir, event_sender)
            .await?;

        // Step 2: Verify package identity matches expected
        Self::verify_package_identity(staging_dir, &manifest, event_sender)?;

        // Step 3: Verify file count consistency
        self.verify_file_count(staging_dir, validation_result, event_sender)
            .await?;

        // Step 4: Verify directory structure is safe
        self.verify_directory_structure(staging_dir, event_sender)
            .await?;

        // Mark as validated
        staging_dir.is_validated = true;

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

    /// Verify manifest exists and parse it
    async fn verify_and_parse_manifest(
        &self,
        staging_dir: &StagingDirectory,
        event_sender: Option<&EventSender>,
    ) -> Result<Manifest, Error> {
        let manifest_path = staging_dir.path.join("manifest.toml");
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "DEBUG: Checking for manifest at: {}",
                    manifest_path.display()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        // Add a small delay to ensure filesystem visibility after extraction
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let manifest_exists = tokio::fs::metadata(&manifest_path).await.is_ok();
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "DEBUG: Manifest exists check: {} -> {}",
                    manifest_path.display(),
                    manifest_exists
                ),
                context: std::collections::HashMap::new(),
            });
        }

        if !manifest_exists {
            if let Some(sender) = event_sender {
                let _ = sender.send(Event::DebugLog {
                    message: "DEBUG: Manifest not found after delay, listing staging directory contents again".to_string(),
                    context: std::collections::HashMap::new(),
                });
                self.debug_list_directory_contents(&staging_dir.path, sender)
                    .await;
            }
            return Err(InstallError::InvalidPackageFile {
                path: staging_dir.path.display().to_string(),
                message: "missing manifest.toml in extracted package".to_string(),
            }
            .into());
        }

        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: "DEBUG: About to parse manifest file".to_string(),
                context: std::collections::HashMap::new(),
            });
        }

        let manifest = Manifest::from_file(&manifest_path).await.map_err(|e| {
            if let Some(sender) = event_sender {
                let _ = sender.send(Event::DebugLog {
                    message: format!("DEBUG: Manifest parsing failed: {e}"),
                    context: std::collections::HashMap::new(),
                });
            }
            InstallError::InvalidPackageFile {
                path: manifest_path.display().to_string(),
                message: format!("invalid manifest.toml: {e}"),
            }
        })?;

        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "DEBUG: Manifest parsed successfully: {}",
                    manifest.package.name
                ),
                context: std::collections::HashMap::new(),
            });
        }

        manifest
            .validate()
            .map_err(|e| InstallError::InvalidPackageFile {
                path: manifest_path.display().to_string(),
                message: format!("manifest validation failed: {e}"),
            })?;

        Ok(manifest)
    }

    /// Debug helper to list directory contents
    async fn debug_list_directory_contents(&self, path: &Path, sender: &EventSender) {
        if let Ok(mut entries) = tokio::fs::read_dir(path).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let _ = sender.send(Event::DebugLog {
                    message: format!("DEBUG: Found file: {}", entry.file_name().to_string_lossy()),
                    context: std::collections::HashMap::new(),
                });
            }
        }
    }

    /// Verify package identity matches expected
    fn verify_package_identity(
        staging_dir: &StagingDirectory,
        manifest: &Manifest,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "DEBUG: Checking package identity: expected '{}', found '{}'",
                    staging_dir.package_id.name, manifest.package.name
                ),
                context: std::collections::HashMap::new(),
            });
        }

        if manifest.package.name != staging_dir.package_id.name {
            if let Some(sender) = event_sender {
                let _ = sender.send(Event::DebugLog {
                    message: "DEBUG: Package name mismatch error!".to_string(),
                    context: std::collections::HashMap::new(),
                });
            }
            return Err(InstallError::InvalidPackageFile {
                path: staging_dir.path.display().to_string(),
                message: format!(
                    "package name mismatch: expected '{}', found '{}'",
                    staging_dir.package_id.name, manifest.package.name
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Verify file count matches validation result
    async fn verify_file_count(
        &self,
        staging_dir: &StagingDirectory,
        validation_result: &ValidationResult,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        let actual_file_count = self.count_files(&staging_dir.path).await?;
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "DEBUG: File count check: expected {}, found {}",
                    validation_result.file_count, actual_file_count
                ),
                context: std::collections::HashMap::new(),
            });
        }

        if actual_file_count != validation_result.file_count {
            if let Some(sender) = event_sender {
                let _ = sender.send(Event::DebugLog {
                    message: "DEBUG: File count mismatch error!".to_string(),
                    context: std::collections::HashMap::new(),
                });
            }
            return Err(InstallError::InvalidPackageFile {
                path: staging_dir.path.display().to_string(),
                message: format!(
                    "file count mismatch: expected {}, found {}",
                    validation_result.file_count, actual_file_count
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Verify directory structure is secure
    async fn verify_directory_structure(
        &self,
        staging_dir: &StagingDirectory,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: "DEBUG: Starting directory structure validation".to_string(),
                context: std::collections::HashMap::new(),
            });
        }

        self.validate_directory_structure(&staging_dir.path).await?;

        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: "DEBUG: Directory structure validation complete".to_string(),
                context: std::collections::HashMap::new(),
            });
        }

        Ok(())
    }

    /// Validate directory structure for security
    async fn validate_directory_structure(&self, path: &Path) -> Result<(), Error> {
        let mut entries = fs::read_dir(path)
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "read_staging_dir".to_string(),
                path: path.display().to_string(),
                message: e.to_string(),
            })?;

        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "read_staging_entry".to_string(),
                    path: path.display().to_string(),
                    message: e.to_string(),
                })?
        {
            let entry_path = entry.path();
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            // Check for suspicious file names
            if file_name_str.starts_with('.') && file_name_str != "." && file_name_str != ".." {
                // Allow common hidden files and macOS metadata files
                if !matches!(file_name_str.as_ref(), ".gitkeep" | ".gitignore") 
                    && !file_name_str.starts_with("._") // macOS resource forks/metadata
                    && !file_name_str.starts_with(".DS_Store")
                // macOS finder metadata
                {
                    return Err(InstallError::InvalidPackageFile {
                        path: entry_path.display().to_string(),
                        message: format!("suspicious hidden file: {file_name_str}"),
                    }
                    .into());
                }
            }

            // Check for overly long file names
            if file_name_str.len() > 255 {
                return Err(InstallError::InvalidPackageFile {
                    path: entry_path.display().to_string(),
                    message: "file name too long".to_string(),
                }
                .into());
            }

            // Recursively check subdirectories
            if entry_path.is_dir() {
                Box::pin(self.validate_directory_structure(&entry_path)).await?;
            }
        }

        Ok(())
    }

    /// Count files in a directory recursively
    async fn count_files(&self, path: &Path) -> Result<usize, Error> {
        let mut count = 0;
        let mut entries = fs::read_dir(path)
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "count_files".to_string(),
                path: path.display().to_string(),
                message: e.to_string(),
            })?;

        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "count_files_entry".to_string(),
                    path: path.display().to_string(),
                    message: e.to_string(),
                })?
        {
            count += 1;
            let entry_path = entry.path();

            if entry_path.is_dir() {
                count += Box::pin(self.count_files(&entry_path)).await?;
            }
        }

        Ok(count)
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
    pub is_validated: bool,
}

impl StagingDirectory {
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

    /// Get the manifest path
    #[must_use]
    pub fn manifest_path(&self) -> PathBuf {
        self.path.join("manifest.toml")
    }

    /// Get the files directory path
    #[must_use]
    pub fn files_path(&self) -> PathBuf {
        self.path.join("files")
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_staging_directory_creation() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        // Create staging manager with custom base path for testing
        let staging_base = temp.path().join("staging");

        let manager = StagingManager::new(store, staging_base.clone())
            .await
            .unwrap();

        let package_id = PackageId::new(
            "test-pkg".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );

        let staging_dir = manager.create_staging_dir(&package_id).await.unwrap();

        assert!(staging_dir.path.exists());
        assert!(staging_dir.path.starts_with(&staging_base));
        assert!(!staging_dir.is_validated);

        // Cleanup
        staging_dir.cleanup().await.unwrap();
        assert!(!staging_dir.path.exists());
    }

    #[tokio::test]
    async fn test_staging_guard() {
        let temp = tempdir().unwrap();
        let staging_path = temp.path().join("test-staging");
        fs::create_dir_all(&staging_path).await.unwrap();

        let package_id = PackageId::new(
            "test-pkg".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );

        let staging_dir = StagingDirectory {
            path: staging_path.clone(),
            package_id,
            staging_id: Uuid::new_v4(),
            is_validated: false,
        };

        // Test auto-cleanup with guard
        {
            let _guard = StagingGuard::new(staging_dir);
            assert!(staging_path.exists());
        }

        // Give async cleanup time to run
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn test_file_counting() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        let staging_base = temp.path().join("staging");

        let manager = StagingManager::new(store, staging_base).await.unwrap();

        // Create test directory structure
        let test_dir = temp.path().join("test");
        fs::create_dir_all(&test_dir).await.unwrap();
        fs::write(test_dir.join("file1.txt"), b"content1")
            .await
            .unwrap();
        fs::write(test_dir.join("file2.txt"), b"content2")
            .await
            .unwrap();

        let sub_dir = test_dir.join("subdir");
        fs::create_dir_all(&sub_dir).await.unwrap();
        fs::write(sub_dir.join("file3.txt"), b"content3")
            .await
            .unwrap();

        let count = manager.count_files(&test_dir).await.unwrap();
        assert_eq!(count, 4); // 2 files + 1 subdir + 1 file in subdir
    }

    #[tokio::test]
    async fn test_directory_structure_validation() {
        let temp = tempdir().unwrap();
        let store = PackageStore::new(temp.path().to_path_buf());

        let staging_base = temp.path().join("staging");

        let manager = StagingManager::new(store, staging_base).await.unwrap();

        // Test valid structure
        let valid_dir = temp.path().join("valid");
        fs::create_dir_all(&valid_dir).await.unwrap();
        fs::write(valid_dir.join("normal.txt"), b"content")
            .await
            .unwrap();
        fs::write(valid_dir.join(".gitkeep"), b"").await.unwrap();

        let result = manager.validate_directory_structure(&valid_dir).await;
        assert!(result.is_ok());

        // Test invalid structure (suspicious hidden file)
        let invalid_dir = temp.path().join("invalid");
        fs::create_dir_all(&invalid_dir).await.unwrap();
        fs::write(invalid_dir.join(".suspicious"), b"content")
            .await
            .unwrap();

        let result = manager.validate_directory_structure(&invalid_dir).await;
        assert!(result.is_err());
    }
}
