//! File restoration and healing logic

use crate::types::HealingContext;
use sps2_errors::{Error, OpsError};
use sps2_events::{Event, EventSender};
use sps2_hash::Hash;
use sps2_state::queries;
use sps2_store::StoredPackage;
use std::collections::HashMap;
use std::path::Path;

/// Restore a missing file from the package store
///
/// # Errors
///
/// Returns an error if:
/// - Package information cannot be retrieved from database
/// - Package content is missing from store
/// - File restoration fails
pub async fn restore_missing_file(
    ctx: &HealingContext<'_>,
    package_name: &str,
    package_version: &str,
    file_path: &str,
) -> Result<(), Error> {
    // Get package hash from database
    let mut state_tx = ctx.state_manager.begin_transaction().await?;
    let state_id = ctx.state_manager.get_active_state().await?;
    let packages = queries::get_state_packages(&mut state_tx, &state_id).await?;
    state_tx.commit().await?;

    // Find the specific package
    let package = packages
        .iter()
        .find(|p| p.name == package_name && p.version == package_version)
        .ok_or_else(|| OpsError::OperationFailed {
            message: format!("Package {package_name}-{package_version} not found in state"),
        })?;

    // Load package from store
    let package_hash = Hash::from_hex(&package.hash).map_err(|e| OpsError::OperationFailed {
        message: format!("Invalid package hash: {e}"),
    })?;
    let store_path = ctx.store.package_path(&package_hash);

    if !store_path.exists() {
        return Err(OpsError::OperationFailed {
            message: format!(
                "Package content missing from store for {package_name}-{package_version}"
            ),
        }
        .into());
    }

    let stored_package = StoredPackage::load(&store_path).await?;
    let source_file = stored_package.files_path().join(file_path);

    if !source_file.exists() {
        return Err(OpsError::OperationFailed {
            message: format!(
                "File {file_path} not found in stored package {package_name}-{package_version}"
            ),
        }
        .into());
    }

    // Determine target path
    let live_path = ctx.state_manager.live_path();
    let target_path = live_path.join(file_path);

    // Create parent directories if needed
    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to create parent directories: {e}"),
            })?;
    }

    // Get source file metadata for permissions
    let metadata = tokio::fs::metadata(&source_file).await?;

    // Restore the file based on its type
    if metadata.is_dir() {
        // Create directory
        tokio::fs::create_dir_all(&target_path)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to create directory {}: {e}", target_path.display()),
            })?;
    } else if metadata.is_symlink() {
        // Read and recreate symlink
        let link_target = tokio::fs::read_link(&source_file).await?;
        tokio::fs::symlink(&link_target, &target_path)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to create symlink {}: {e}", target_path.display()),
            })?;
    } else {
        // Regular file - use APFS clonefile for efficiency on macOS
        #[cfg(target_os = "macos")]
        {
            sps2_root::clone_directory(&source_file, &target_path)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to clone file {}: {e}", target_path.display()),
                })?;
        }

        #[cfg(not(target_os = "macos"))]
        {
            tokio::fs::copy(&source_file, &target_path)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to copy file {}: {e}", target_path.display()),
                })?;
        }
    }

    // Restore permissions (on Unix-like systems)
    #[cfg(unix)]
    {
        let permissions = metadata.permissions();
        tokio::fs::set_permissions(&target_path, permissions)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to set permissions: {e}"),
            })?;
    }

    Ok(())
}

/// Heal a corrupted file by restoring it from the package store
///
/// # Errors
///
/// Returns an error if:
/// - Package information cannot be retrieved
/// - File restoration fails
/// - Store content is missing
pub async fn heal_corrupted_file(
    ctx: &HealingContext<'_>,
    package_name: &str,
    package_version: &str,
    file_path: &str,
    expected_hash: &str,
    actual_hash: &str,
) -> Result<(), Error> {
    let live_path = ctx.state_manager.live_path();
    let full_path = live_path.join(file_path);

    // First, check if this might be a legitimate user modification
    if is_user_modified_file(ctx.tx, &full_path, file_path).await? {
        // Preserve user modifications
        let _ = ctx.tx.send(Event::DebugLog {
            message: format!(
                "Preserving user-modified file: {file_path} (hash mismatch: expected {expected_hash}, got {actual_hash})"
            ),
            context: HashMap::default(),
        });
        return Ok(());
    }

    // Get package from database to find store location
    let mut state_tx = ctx.state_manager.begin_transaction().await?;
    let state_id = ctx.state_manager.get_active_state().await?;
    let packages = queries::get_state_packages(&mut state_tx, &state_id).await?;
    state_tx.commit().await?;

    let package = packages
        .iter()
        .find(|p| p.name == package_name && p.version == package_version)
        .ok_or_else(|| OpsError::OperationFailed {
            message: format!("Package {package_name}-{package_version} not found in state"),
        })?;

    // Load package from store
    let package_hash = Hash::from_hex(&package.hash).map_err(|e| OpsError::OperationFailed {
        message: format!("Invalid package hash: {e}"),
    })?;
    let store_path = ctx.store.package_path(&package_hash);

    if !store_path.exists() {
        return Err(OpsError::OperationFailed {
            message: format!(
                "Package content missing from store for {package_name}-{package_version}"
            ),
        }
        .into());
    }

    let stored_package = StoredPackage::load(&store_path).await?;
    let source_file = stored_package.files_path().join(file_path);

    if !source_file.exists() {
        return Err(OpsError::OperationFailed {
            message: format!(
                "File {file_path} not found in stored package {package_name}-{package_version}"
            ),
        }
        .into());
    }

    // Verify the source file has the expected hash
    let source_hash = Hash::hash_file(&source_file).await?;
    if source_hash.to_hex() != expected_hash {
        return Err(OpsError::OperationFailed {
            message: format!(
                "Source file in store also corrupted for {file_path} (expected {expected_hash}, got {})",
                source_hash.to_hex()
            ),
        }
        .into());
    }

    // Backup the corrupted file before replacing
    let backup_path = full_path.with_extension("corrupted.backup");
    tokio::fs::rename(&full_path, &backup_path)
        .await
        .map_err(|e| OpsError::OperationFailed {
            message: format!("Failed to backup corrupted file: {e}"),
        })?;

    // Restore the file from store
    let metadata = tokio::fs::metadata(&source_file).await?;

    if metadata.is_symlink() {
        // Recreate symlink
        let link_target = tokio::fs::read_link(&source_file).await?;
        tokio::fs::symlink(&link_target, &full_path)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to restore symlink {}: {e}", full_path.display()),
            })?;
    } else {
        // Regular file - use APFS clonefile for efficiency on macOS
        #[cfg(target_os = "macos")]
        {
            sps2_root::clone_directory(&source_file, &full_path)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to restore file {}: {e}", full_path.display()),
                })?;
        }

        #[cfg(not(target_os = "macos"))]
        {
            tokio::fs::copy(&source_file, &full_path)
                .await
                .map_err(|e| OpsError::OperationFailed {
                    message: format!("Failed to restore file {}: {e}", full_path.display()),
                })?;
        }
    }

    // Restore permissions
    #[cfg(unix)]
    {
        let permissions = metadata.permissions();
        tokio::fs::set_permissions(&full_path, permissions)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to restore permissions: {e}"),
            })?;
    }

    // Emit success event
    let _ = ctx.tx.send(Event::DebugLog {
        message: format!(
            "Restored corrupted file: {file_path} (backup saved as {})",
            backup_path.display()
        ),
        context: HashMap::default(),
    });

    Ok(())
}

/// Check if a file appears to be user-modified
///
/// This is a heuristic check based on:
/// - File modification time vs package installation time
/// - Common user-modifiable file patterns
/// - File location and type
pub async fn is_user_modified_file(
    tx: &EventSender,
    full_path: &Path,
    relative_path: &str,
) -> Result<bool, Error> {
    // Common patterns for user-modifiable files
    const USER_MODIFIABLE_PATTERNS: &[&str] = &[
        // Configuration files
        ".conf",
        ".config",
        ".ini",
        ".json",
        ".yaml",
        ".yml",
        ".toml",
        // Shell configuration
        ".bashrc",
        ".zshrc",
        ".profile",
        ".bash_profile",
        // Environment files
        ".env",
        ".envrc",
        // User data
        ".db",
        ".sqlite",
        ".sqlite3",
    ];

    // Check if file matches user-modifiable patterns
    let path_str = relative_path.to_lowercase();
    for pattern in USER_MODIFIABLE_PATTERNS {
        if path_str.ends_with(pattern)
            || path_str.contains("/etc/")
            || path_str.contains("/config/")
        {
            // These files are commonly modified by users
            let _ = tx.send(Event::DebugLog {
                message: format!("File {relative_path} matches user-modifiable pattern"),
                context: HashMap::default(),
            });
            return Ok(true);
        }
    }

    // Check file metadata for recent modifications
    if let Ok(metadata) = tokio::fs::metadata(full_path).await {
        if let Ok(modified) = metadata.modified() {
            // If file was modified very recently (within last hour), it might be user-modified
            if let Ok(elapsed) = modified.elapsed() {
                if elapsed.as_secs() < 3600 {
                    let _ = tx.send(Event::DebugLog {
                        message: format!(
                            "File {relative_path} was modified recently ({} seconds ago)",
                            elapsed.as_secs()
                        ),
                        context: HashMap::default(),
                    });
                    return Ok(true);
                }
            }
        }
    }

    Ok(false)
}
