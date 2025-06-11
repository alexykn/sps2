//! Rollback operations for atomic installations

use sps2_errors::{Error, InstallError};
use sps2_state::StateManager;
use std::path::Path;
use tokio::fs;
use uuid::Uuid;

/// Rollback to a previous state
///
/// # Errors
///
/// Returns an error if the target state doesn't exist, filesystem swap fails,
/// database update fails, or archiving fails.
pub async fn rollback_to_state(
    state_manager: &StateManager,
    live_path: &Path,
    target_state_id: Uuid,
) -> Result<(), Error> {
    // Get the current state ID before we start the rollback
    // This is what's actually in /opt/pm/live right now
    let current_state_id = state_manager.get_current_state_id().await?;

    let target_path = state_manager.get_state_path(target_state_id)?;

    // Use true atomic swap to exchange target state with live directory
    sps2_root::atomic_swap(&target_path, live_path)
        .await
        .map_err(|e| InstallError::FilesystemError {
            operation: "rollback_atomic_swap".to_string(),
            path: target_path.display().to_string(),
            message: e.to_string(),
        })?;

    // Archive the old live directory (now in target_path) to preserve it
    let archive_path = state_manager.get_state_path(current_state_id)?;

    // Check if archive already exists and handle collision
    if !archive_path.exists() {
        // Archive normally if state doesn't exist
        fs::rename(&target_path, &archive_path).await.map_err(|e| {
            InstallError::FilesystemError {
                operation: "archive_old_live".to_string(),
                path: archive_path.display().to_string(),
                message: e.to_string(),
            }
        })?;
    } else {
        // State already exists - archive with timestamp to avoid collision
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let alternative_path = state_manager
            .state_path()
            .join(format!("{}-{}", current_state_id, timestamp));

        fs::rename(&target_path, &alternative_path)
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "archive_old_live_alternative".to_string(),
                path: alternative_path.display().to_string(),
                message: e.to_string(),
            })?;
    }

    // Update active state in database
    state_manager.set_active_state(target_state_id).await?;

    Ok(())
}
