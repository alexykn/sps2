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
    eprintln!(
        "DEBUG: restore_missing_file starting for {}/{} - {}",
        package_name, package_version, file_path
    );

    let result = restore_missing_file_impl(ctx, package_name, package_version, file_path).await;

    // Errors are already logged inside restore_missing_file_impl

    result
}

async fn restore_missing_file_impl(
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

    // For file-level packages, we need to get the file hash from the database
    let source_file = if stored_package.has_file_hashes() {
        // Get file hash from database
        let mut state_tx = ctx.state_manager.begin_transaction().await?;
        let state_id = ctx.state_manager.get_active_state().await?;

        // Get the file entry to find its hash
        // First try the current state
        let mut file_entries = sps2_state::queries::get_package_file_entries_by_name(
            &mut state_tx,
            &state_id,
            package_name,
            package_version,
        )
        .await?;

        // If not found in current state, look for any package with same name/version
        if file_entries.is_empty() {
            // This is a workaround - ideally file entries should be copied to new states
            file_entries = sps2_state::queries::get_package_file_entries_all_states(
                &mut state_tx,
                package_name,
                package_version,
            )
            .await?;
        }

        state_tx.commit().await?;

        // Find the specific file entry
        let file_entry = file_entries
            .iter()
            .find(|entry| entry.relative_path == file_path)
            .ok_or_else(|| OpsError::OperationFailed {
                message: format!(
                    "File {file_path} not found in database for {package_name}-{package_version}"
                ),
            })?;

        // Get the file from the file store
        let file_hash_obj =
            Hash::from_hex(&file_entry.file_hash).map_err(|e| OpsError::OperationFailed {
                message: format!("Invalid file hash in database: {e}"),
            })?;

        let file_store_path = ctx.store.file_path(&file_hash_obj);

        if !file_store_path.exists() {
            return Err(OpsError::OperationFailed {
                message: format!(
                    "File content missing from file store for hash {}",
                    file_entry.file_hash
                ),
            }
            .into());
        }

        file_store_path
    } else {
        // Legacy package - file is in the package's files directory
        let source_file = stored_package.files_path().join(file_path);

        if !source_file.exists() {
            return Err(OpsError::OperationFailed {
                message: format!(
                    "File {file_path} not found in stored package {package_name}-{package_version}"
                ),
            }
            .into());
        }

        source_file
    };

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

    eprintln!(
        "DEBUG: Successfully restored {} to {}",
        file_path,
        target_path.display()
    );

    // Clear the mtime trackers for this package so the healed file is re-verified
    let _ = ctx.tx.send(Event::DebugLog {
        message: format!("About to clear mtime trackers for {package_name}-{package_version}"),
        context: HashMap::default(),
    });

    let mut state_tx = ctx.state_manager.begin_transaction().await?;
    let cleared = sps2_state::queries::clear_package_mtime_trackers(
        &mut state_tx,
        package_name,
        package_version,
    )
    .await?;
    state_tx.commit().await?;

    let _ = ctx.tx.send(Event::DebugLog {
        message: format!(
            "Cleared {cleared} mtime tracker entries for {package_name}-{package_version}"
        ),
        context: HashMap::default(),
    });

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

    // Debug logging
    let _ = ctx.tx.send(Event::DebugLog {
        message: format!(
            "Starting heal_corrupted_file for {file_path} (expected: {expected_hash}, actual: {actual_hash})"
        ),
        context: HashMap::default(),
    });

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

    let _ = ctx.tx.send(Event::DebugLog {
        message: format!("File {file_path} is not user-modified, proceeding with healing"),
        context: HashMap::default(),
    });

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

    // For file-level packages, get the file from the file store
    let source_file = if stored_package.has_file_hashes() {
        // Get the expected hash and find the file in the file store
        let expected_hash_obj =
            Hash::from_hex(expected_hash).map_err(|e| OpsError::OperationFailed {
                message: format!("Invalid expected hash: {e}"),
            })?;

        // Get the file path from the store
        let file_store_path = ctx.store.file_path(&expected_hash_obj);

        if !file_store_path.exists() {
            return Err(OpsError::OperationFailed {
                message: format!("File content missing from file store for hash {expected_hash}"),
            }
            .into());
        }

        file_store_path
    } else {
        // Legacy package - file is in the package's files directory
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

        source_file
    };

    // Remove the corrupted file
    tokio::fs::remove_file(&full_path)
        .await
        .map_err(|e| OpsError::OperationFailed {
            message: format!("Failed to remove corrupted file: {e}"),
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

    // Clear the mtime trackers for this package so the healed file is re-verified
    let mut state_tx = ctx.state_manager.begin_transaction().await?;
    let cleared = sps2_state::queries::clear_package_mtime_trackers(
        &mut state_tx,
        package_name,
        package_version,
    )
    .await?;
    state_tx.commit().await?;

    // Emit success event
    let _ = ctx.tx.send(Event::DebugLog {
        message: format!(
            "Restored corrupted file: {file_path}, cleared {cleared} mtime tracker entries"
        ),
        context: HashMap::default(),
    });

    Ok(())
}

/// Check if a file appears to be user-modified
///
/// In sps2, the /opt/pm/live directory should be immutable except for symlinks.
/// Any modification to regular files is considered corruption.
pub async fn is_user_modified_file(
    tx: &EventSender,
    full_path: &Path,
    relative_path: &str,
) -> Result<bool, Error> {
    // Check if this is a symlink - symlinks are the only allowed user modifications
    if let Ok(metadata) = tokio::fs::symlink_metadata(full_path).await {
        if metadata.is_symlink() {
            let _ = tx.send(Event::DebugLog {
                message: format!(
                    "File {relative_path} is a symlink - preserving user modification"
                ),
                context: HashMap::default(),
            });
            return Ok(true);
        }
    }

    // All other modifications in /opt/pm/live are considered corruption, not user modifications
    Ok(false)
}
