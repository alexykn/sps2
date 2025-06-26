//! Orphaned file handling logic

use crate::types::{OrphanedFileAction, OrphanedFileCategory};
use sps2_errors::{Error, OpsError};
use sps2_events::{Event, EventSender};
use sps2_state::StateManager;
use std::collections::HashMap;
use std::path::Path;

/// Handle an orphaned file based on configuration and category
///
/// # Errors
///
/// Returns an error if:
/// - File operations fail
/// - Backup directory creation fails
pub async fn handle_orphaned_file(
    state_manager: &StateManager,
    tx: &EventSender,
    file_path: &str,
    category: &OrphanedFileCategory,
    config: &sps2_config::Config,
) -> Result<(), Error> {
    let live_path = state_manager.live_path();
    let full_path = live_path.join(file_path);

    // Determine action based on configuration and category
    let action = determine_orphaned_file_action(category, config);

    // Emit event about the action
    let _ = tx.send(Event::DebugLog {
        message: format!(
            "Handling orphaned file: {file_path} (category: {category:?}, action: {action:?})"
        ),
        context: HashMap::default(),
    });

    match action {
        OrphanedFileAction::Preserve => {
            // Just log that we're preserving it
            let _ = tx.send(Event::DebugLog {
                message: format!("Preserving orphaned file: {file_path}"),
                context: HashMap::default(),
            });
            Ok(())
        }
        OrphanedFileAction::Remove => remove_orphaned_file(tx, &full_path, file_path).await,
        OrphanedFileAction::Backup => {
            backup_and_remove_orphaned_file(
                tx,
                &full_path,
                file_path,
                &config.verification.orphaned_backup_dir,
            )
            .await
        }
    }
}

/// Determine what action to take for an orphaned file
pub fn determine_orphaned_file_action(
    category: &OrphanedFileCategory,
    config: &sps2_config::Config,
) -> OrphanedFileAction {
    // System files are always preserved
    if matches!(category, OrphanedFileCategory::System) {
        return OrphanedFileAction::Preserve;
    }

    // User-created files respect configuration
    if matches!(category, OrphanedFileCategory::UserCreated)
        && config.verification.preserve_user_files
    {
        return OrphanedFileAction::Preserve;
    }

    // Parse the configured action
    match config.verification.orphaned_file_action.as_str() {
        "remove" => OrphanedFileAction::Remove,
        "backup" => OrphanedFileAction::Backup,
        _ => OrphanedFileAction::Preserve, // Default to preserve for safety
    }
}

/// Safely remove an orphaned file
pub async fn remove_orphaned_file(
    tx: &EventSender,
    full_path: &Path,
    relative_path: &str,
) -> Result<(), Error> {
    // Check if it's a directory or file
    let metadata = tokio::fs::metadata(full_path).await?;

    if metadata.is_dir() {
        // For directories, only remove if empty
        match tokio::fs::read_dir(full_path).await {
            Ok(mut entries) => {
                if entries.next_entry().await?.is_some() {
                    // Directory not empty, preserve it
                    let _ = tx.send(Event::DebugLog {
                        message: format!(
                            "Preserving non-empty orphaned directory: {relative_path}"
                        ),
                        context: HashMap::default(),
                    });
                    return Ok(());
                }
                // Directory is empty, safe to remove
                tokio::fs::remove_dir(full_path)
                    .await
                    .map_err(|e| OpsError::OperationFailed {
                        message: format!("Failed to remove empty directory {relative_path}: {e}"),
                    })?;
            }
            Err(e) => {
                return Err(OpsError::OperationFailed {
                    message: format!("Failed to read directory {relative_path}: {e}"),
                }
                .into());
            }
        }
    } else {
        // Regular file or symlink
        tokio::fs::remove_file(full_path)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to remove file {relative_path}: {e}"),
            })?;
    }

    let _ = tx.send(Event::DebugLog {
        message: format!("Removed orphaned file: {relative_path}"),
        context: HashMap::default(),
    });

    Ok(())
}

/// Backup an orphaned file then remove it
pub async fn backup_and_remove_orphaned_file(
    tx: &EventSender,
    full_path: &Path,
    relative_path: &str,
    backup_dir: &Path,
) -> Result<(), Error> {
    // Create backup directory structure
    let backup_path = backup_dir.join(relative_path);
    if let Some(parent) = backup_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to create backup directory: {e}"),
            })?;
    }

    // Move file to backup location
    tokio::fs::rename(full_path, &backup_path)
        .await
        .map_err(|e| OpsError::OperationFailed {
            message: format!(
                "Failed to backup file {relative_path} to {}: {e}",
                backup_path.display()
            ),
        })?;

    let _ = tx.send(Event::DebugLog {
        message: format!(
            "Backed up orphaned file: {relative_path} -> {}",
            backup_path.display()
        ),
        context: HashMap::default(),
    });

    Ok(())
}
