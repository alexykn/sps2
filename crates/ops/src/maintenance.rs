//! System Cleanup and State Management Operations

use crate::{ChangeType, OpChange, OpsCtx, StateInfo};
use sps2_errors::{Error, OpsError};
use sps2_events::{AppEvent, EventEmitter, PackageEvent, StateEvent};
use std::time::Instant;
use uuid::Uuid;

/// Clean up orphaned packages and old states
///
/// # Errors
///
/// Returns an error if cleanup operation fails.
pub async fn cleanup(ctx: &OpsCtx) -> Result<String, Error> {
    let start = Instant::now();

    ctx.emit_event(AppEvent::Package(PackageEvent::CleanupStarting));

    // Clean up old states, respecting the configured retention count
    let cleanup_result = ctx
        .state
        .cleanup(ctx.config.state.retention_count, 30)
        .await?;

    // Run garbage collection on store
    let cleaned_packages = ctx.state.gc_store_with_removal(&ctx.store).await?;

    let duration = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let message = format!(
        "Cleaned up {} old states and {} orphaned packages",
        cleanup_result.states_removed, cleaned_packages
    );

    ctx.emit_event(AppEvent::Package(PackageEvent::CleanupCompleted {
        states_removed: cleanup_result.states_removed,
        packages_removed: cleaned_packages,
        duration_ms: duration,
    }));

    // Update GC timestamp after successful cleanup
    if let Err(e) = update_gc_timestamp().await {
        // Log but don't fail the cleanup operation
        eprintln!("Warning: Failed to update GC timestamp: {e}");
    }

    Ok(message)
}

/// Rollback to a previous state
///
/// # Errors
///
/// Returns an error if:
/// - No previous state exists
/// - Rollback operation fails
pub async fn rollback(ctx: &OpsCtx, target_state: Option<Uuid>) -> Result<StateInfo, Error> {
    let start = Instant::now();

    // If no target specified, rollback to previous state
    let target_id = if let Some(id) = target_state {
        id
    } else {
        let current_id = ctx.state.get_current_state_id().await?;

        ctx.state
            .get_parent_state_id(&current_id)
            .await?
            .ok_or(OpsError::NoPreviousState)?
    };

    ctx.emit_event(AppEvent::State(StateEvent::RollbackStarting {
        target_state: target_id,
    }));

    // Verify target state exists in database
    if !ctx.state.state_exists(&target_id).await? {
        return Err(OpsError::StateNotFound {
            state_id: target_id,
        }
        .into());
    }

    // Verify target state directory exists on filesystem
    let state_path = ctx.state.state_path().join(target_id.to_string());
    if !state_path.exists() {
        return Err(OpsError::StateNotFound {
            state_id: target_id,
        }
        .into());
    }

    // Perform rollback using atomic installer
    let mut atomic_installer =
        sps2_install::AtomicInstaller::new(ctx.state.clone(), ctx.store.clone()).await?;

    atomic_installer.rollback(target_id).await?;

    // Get state information
    let state_info = get_state_info(ctx, target_id).await?;

    let _ = ctx.tx.send(AppEvent::State(StateEvent::RollbackCompleted {
        target_state: target_id,
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
    }));

    Ok(state_info)
}

/// Get history of states
///
/// # Errors
///
/// Returns an error if state history retrieval fails.
pub async fn history(ctx: &OpsCtx) -> Result<Vec<StateInfo>, Error> {
    let states = ctx.state.list_states_detailed().await?;
    let current_id = ctx.state.get_current_state_id().await?;

    let mut state_infos = Vec::new();

    for state in states {
        let state_id = state.state_id();
        let parent_id = state
            .parent_id
            .as_ref()
            .and_then(|p| uuid::Uuid::parse_str(p).ok());

        // Get actual package count for this state
        let package_count = get_state_package_count(ctx, &state_id).await?;

        // Calculate changes from parent state
        let changes = if let Some(parent_id) = parent_id {
            calculate_state_changes(ctx, &parent_id, &state_id).await?
        } else {
            // Root state - all packages are installs
            get_initial_state_changes(ctx, &state_id).await?
        };

        let state_info = StateInfo {
            id: state_id,
            parent: parent_id,
            parent_id,
            timestamp: state.timestamp(),
            operation: state.operation.clone(),
            current: Some(current_id) == Some(state_id),
            package_count,
            total_size: 0, // TODO: Calculate actual size
            changes,
        };

        state_infos.push(state_info);
    }

    // Sort by timestamp (newest first)
    state_infos.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(state_infos)
}

/// Get state information by ID
async fn get_state_info(ctx: &OpsCtx, state_id: Uuid) -> Result<StateInfo, Error> {
    let states = ctx.state.list_states_detailed().await?;
    let current_id = ctx.state.get_current_state_id().await?;

    let state = states
        .iter()
        .find(|s| s.state_id() == state_id)
        .ok_or(OpsError::StateNotFound { state_id })?;

    let parent_id = state
        .parent_id
        .as_ref()
        .and_then(|p| uuid::Uuid::parse_str(p).ok());

    // Get actual package count for this state
    let package_count = get_state_package_count(ctx, &state_id).await?;

    // Calculate changes from parent state
    let changes = if let Some(parent_id) = parent_id {
        calculate_state_changes(ctx, &parent_id, &state_id).await?
    } else {
        // Root state - all packages are installs
        get_initial_state_changes(ctx, &state_id).await?
    };

    Ok(StateInfo {
        id: state_id,
        parent: parent_id,
        parent_id,
        timestamp: state.timestamp(),
        operation: state.operation.clone(),
        current: Some(current_id) == Some(state_id),
        package_count,
        total_size: 0, // TODO: Calculate actual size
        changes,
    })
}

/// Get package count for a specific state
async fn get_state_package_count(ctx: &OpsCtx, state_id: &Uuid) -> Result<usize, Error> {
    let packages = ctx.state.get_state_packages(state_id).await?;
    Ok(packages.len())
}

/// Calculate changes between parent and child states
async fn calculate_state_changes(
    ctx: &OpsCtx,
    parent_id: &Uuid,
    child_id: &Uuid,
) -> Result<Vec<OpChange>, Error> {
    let parent_packages = ctx.state.get_state_packages(parent_id).await?;
    let child_packages = ctx.state.get_state_packages(child_id).await?;

    let mut changes = Vec::new();

    // Convert to sets for easier comparison
    let parent_set: std::collections::HashSet<&String> = parent_packages.iter().collect();
    let child_set: std::collections::HashSet<&String> = child_packages.iter().collect();

    // Find packages that were added (in child but not parent)
    for package in &child_packages {
        if !parent_set.contains(package) {
            // For now, we can't get version info from package names only
            // In a real implementation, we'd need to get full Package objects
            changes.push(OpChange {
                change_type: ChangeType::Install,
                package: package.clone(),
                old_version: None,
                new_version: None, // Would need actual Package data
            });
        }
    }

    // Find packages that were removed (in parent but not child)
    for package in &parent_packages {
        if !child_set.contains(package) {
            changes.push(OpChange {
                change_type: ChangeType::Remove,
                package: package.clone(),
                old_version: None, // Would need actual Package data
                new_version: None,
            });
        }
    }

    // Note: Updates/downgrades would require version comparison
    // which needs full Package objects, not just names

    Ok(changes)
}

/// Get changes for initial state (all packages are installs)
async fn get_initial_state_changes(ctx: &OpsCtx, state_id: &Uuid) -> Result<Vec<OpChange>, Error> {
    let packages = ctx.state.get_state_packages(state_id).await?;
    let mut changes = Vec::new();

    for package in packages {
        changes.push(OpChange {
            change_type: ChangeType::Install,
            package,
            old_version: None,
            new_version: None, // Would need actual Package data
        });
    }

    Ok(changes)
}

/// Update the GC timestamp after successful cleanup
async fn update_gc_timestamp() -> Result<(), Error> {
    let timestamp_path = std::path::Path::new("/opt/pm/.last_gc_timestamp");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    tokio::fs::write(timestamp_path, now.to_string())
        .await
        .map_err(|e| sps2_errors::Error::internal(format!("Failed to write GC timestamp: {e}")))?;

    Ok(())
}
