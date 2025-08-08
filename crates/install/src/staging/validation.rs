//! Content validation logic for staging directories
//!
//! This module provides validation functions for staging directory content,
//! including manifest verification, package identity checks, and security validation.

use crate::ValidationResult;
use sps2_errors::{Error, InstallError};
use sps2_events::{AppEvent, EventEmitter, EventSender, GeneralEvent};
use sps2_types::Manifest;
use std::path::Path;
use tokio::fs;

use super::{
    directory::StagingDirectory,
    utils::{count_files, debug_list_directory_contents},
};

/// Verify manifest exists and parse it
pub async fn verify_and_parse_manifest(
    staging_dir: &StagingDirectory,
    event_sender: Option<&EventSender>,
) -> Result<Manifest, Error> {
    let manifest_path = staging_dir.path.join("manifest.toml");
    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Checking for manifest at: {}",
                manifest_path.display()
            ),
            context: std::collections::HashMap::new(),
        }));
    }

    // Add a small delay to ensure filesystem visibility after extraction
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let manifest_exists = tokio::fs::metadata(&manifest_path).await.is_ok();
    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Manifest exists check: {} -> {}",
                manifest_path.display(),
                manifest_exists
            ),
            context: std::collections::HashMap::new(),
        }));
    }

    if !manifest_exists {
        if let Some(sender) = event_sender {
            sender.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: "DEBUG: Manifest not found after delay, listing staging directory contents again".to_string(),
                context: std::collections::HashMap::new(),
            }));
            debug_list_directory_contents(&staging_dir.path, sender).await;
        }
        return Err(InstallError::InvalidPackageFile {
            path: staging_dir.path.display().to_string(),
            message: "missing manifest.toml in extracted package".to_string(),
        }
        .into());
    }

    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: "DEBUG: About to parse manifest file".to_string(),
            context: std::collections::HashMap::new(),
        }));
    }

    let manifest = sps2_store::manifest_io::read_manifest(&manifest_path)
        .await
        .map_err(|e| {
            if let Some(sender) = event_sender {
                sender.emit(AppEvent::General(GeneralEvent::DebugLog {
                    message: format!("DEBUG: Manifest parsing failed: {e}"),
                    context: std::collections::HashMap::new(),
                }));
            }
            InstallError::InvalidPackageFile {
                path: manifest_path.display().to_string(),
                message: format!("invalid manifest.toml: {e}"),
            }
        })?;

    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Manifest parsed successfully: {}",
                manifest.package.name
            ),
            context: std::collections::HashMap::new(),
        }));
    }

    manifest
        .validate()
        .map_err(|e| InstallError::InvalidPackageFile {
            path: manifest_path.display().to_string(),
            message: format!("manifest validation failed: {e}"),
        })?;

    Ok(manifest)
}

/// Verify package identity matches expected
pub fn verify_package_identity(
    staging_dir: &StagingDirectory,
    manifest: &Manifest,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: Checking package identity: expected '{}', found '{}'",
                staging_dir.package_id.name, manifest.package.name
            ),
            context: std::collections::HashMap::new(),
        }));
    }

    if manifest.package.name != staging_dir.package_id.name {
        if let Some(sender) = event_sender {
            sender.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: "DEBUG: Package name mismatch error!".to_string(),
                context: std::collections::HashMap::new(),
            }));
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
pub async fn verify_file_count(
    staging_dir: &StagingDirectory,
    validation_result: &ValidationResult,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    let actual_file_count = count_files(&staging_dir.path).await?;
    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "DEBUG: File count check: expected {}, found {}",
                validation_result.file_count, actual_file_count
            ),
            context: std::collections::HashMap::new(),
        }));
    }

    if actual_file_count != validation_result.file_count {
        if let Some(sender) = event_sender {
            sender.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: "DEBUG: File count mismatch error!".to_string(),
                context: std::collections::HashMap::new(),
            }));
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
pub async fn verify_directory_structure(
    staging_dir: &StagingDirectory,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: "DEBUG: Starting directory structure validation".to_string(),
            context: std::collections::HashMap::new(),
        }));
    }

    validate_directory_structure(&staging_dir.path).await?;

    if let Some(sender) = event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: "DEBUG: Directory structure validation complete".to_string(),
            context: std::collections::HashMap::new(),
        }));
    }

    Ok(())
}

/// Validate directory structure for security
pub async fn validate_directory_structure(path: &Path) -> Result<(), Error> {
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

        // Hidden files are allowed - modern packages often contain legitimate hidden files

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
            Box::pin(validate_directory_structure(&entry_path)).await?;
        }
    }

    Ok(())
}
