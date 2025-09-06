//! System Cleanup and State Management Operations

use crate::{ChangeType, OpChange, OpsCtx, StateInfo};
use sps2_errors::{Error, OpsError};
use sps2_events::{AppEvent, EventEmitter, GeneralEvent, PackageEvent, StateEvent};
use std::time::Instant;
use uuid::Uuid;

/// Clean up orphaned packages and old states
///
/// # Errors
///
/// Returns an error if cleanup operation fails.
pub async fn cleanup(ctx: &OpsCtx) -> Result<String, Error> {
    let start = Instant::now();

    ctx.emit(AppEvent::Package(PackageEvent::CleanupStarting));

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

    ctx.emit(AppEvent::Package(PackageEvent::CleanupCompleted {
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

    // Check mode: preview what would be rolled back
    if ctx.check_mode {
        return preview_rollback(ctx, target_state).await;
    }

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

    ctx.emit(AppEvent::State(StateEvent::RollbackExecuting {
        from: ctx.state.get_current_state_id().await?,
        to: target_id,
        packages_affected: 0, // Will be updated during execution
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

    // Calculate changes BEFORE rollback (current -> target)
    let current_id = ctx.state.get_current_state_id().await?;
    let rollback_changes = calculate_state_changes(ctx, &current_id, &target_id).await?;

    // Perform rollback using atomic installer
    let mut atomic_installer =
        sps2_install::AtomicInstaller::new(ctx.state.clone(), ctx.store.clone()).await?;

    atomic_installer.rollback(target_id).await?;

    // Get state information with pre-calculated changes
    let state_info = get_rollback_state_info_with_changes(ctx, target_id, rollback_changes).await?;

    let _ = ctx.tx.send(AppEvent::State(StateEvent::RollbackCompleted {
        from: ctx.state.get_current_state_id().await?,
        to: target_id,
        duration: start.elapsed(),
        packages_reverted: 0, // TODO: Track actual packages reverted
    }));

    Ok(state_info)
}

/// Preview what would be rolled back without executing
#[allow(clippy::too_many_lines)]
async fn preview_rollback(ctx: &OpsCtx, target_state: Option<Uuid>) -> Result<StateInfo, Error> {
    use std::collections::HashMap;

    // Resolve target state (same logic as main rollback)
    let target_id = if let Some(id) = target_state {
        id
    } else {
        let current_id = ctx.state.get_current_state_id().await?;
        ctx.state
            .get_parent_state_id(&current_id)
            .await?
            .ok_or(OpsError::NoPreviousState)?
    };

    // Validate target state exists (same validation as main rollback)
    if !ctx.state.state_exists(&target_id).await? {
        return Err(OpsError::StateNotFound {
            state_id: target_id,
        }
        .into());
    }

    let state_path = ctx.state.state_path().join(target_id.to_string());
    if !state_path.exists() {
        return Err(OpsError::StateNotFound {
            state_id: target_id,
        }
        .into());
    }

    // Calculate changes (current -> target)
    let current_id = ctx.state.get_current_state_id().await?;
    let changes = calculate_state_changes(ctx, &current_id, &target_id).await?;

    // Emit preview events for each change
    let mut added_count = 0;
    let mut removed_count = 0;
    let mut updated_count = 0;

    for change in &changes {
        let (action, change_type, details) = match change.change_type {
            ChangeType::Install => {
                added_count += 1;
                let version = change
                    .new_version
                    .as_ref()
                    .map_or_else(|| "unknown".to_string(), ToString::to_string);
                (
                    format!("Would add {} {}", change.package, version),
                    "add",
                    HashMap::from([
                        ("package".to_string(), change.package.clone()),
                        ("new_version".to_string(), version),
                    ]),
                )
            }
            ChangeType::Remove => {
                removed_count += 1;
                let version = change
                    .old_version
                    .as_ref()
                    .map_or_else(|| "unknown".to_string(), ToString::to_string);
                (
                    format!("Would remove {} {}", change.package, version),
                    "remove",
                    HashMap::from([
                        ("package".to_string(), change.package.clone()),
                        ("current_version".to_string(), version),
                    ]),
                )
            }
            ChangeType::Update => {
                updated_count += 1;
                let old_version = change
                    .old_version
                    .as_ref()
                    .map_or_else(|| "unknown".to_string(), ToString::to_string);
                let new_version = change
                    .new_version
                    .as_ref()
                    .map_or_else(|| "unknown".to_string(), ToString::to_string);
                (
                    format!(
                        "Would update {} {} → {}",
                        change.package, old_version, new_version
                    ),
                    "update",
                    HashMap::from([
                        ("package".to_string(), change.package.clone()),
                        ("current_version".to_string(), old_version),
                        ("target_version".to_string(), new_version),
                    ]),
                )
            }
            ChangeType::Downgrade => {
                updated_count += 1;
                let old_version = change
                    .old_version
                    .as_ref()
                    .map_or_else(|| "unknown".to_string(), ToString::to_string);
                let new_version = change
                    .new_version
                    .as_ref()
                    .map_or_else(|| "unknown".to_string(), ToString::to_string);
                (
                    format!(
                        "Would downgrade {} {} → {}",
                        change.package, old_version, new_version
                    ),
                    "downgrade",
                    HashMap::from([
                        ("package".to_string(), change.package.clone()),
                        ("current_version".to_string(), old_version),
                        ("target_version".to_string(), new_version),
                    ]),
                )
            }
        };

        let mut event_details = details;
        event_details.insert("target_state".to_string(), target_id.to_string());
        event_details.insert("change_type".to_string(), change_type.to_string());

        ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
            operation: "rollback".to_string(),
            action,
            details: event_details,
        }));
    }

    // Emit summary
    let total_changes = changes.len();
    let mut categories = HashMap::new();
    if added_count > 0 {
        categories.insert("packages_added".to_string(), added_count);
    }
    if removed_count > 0 {
        categories.insert("packages_removed".to_string(), removed_count);
    }
    if updated_count > 0 {
        categories.insert("packages_updated".to_string(), updated_count);
    }

    ctx.emit(AppEvent::General(GeneralEvent::CheckModeSummary {
        operation: "rollback".to_string(),
        total_changes,
        categories,
    }));

    // Get target state info for preview (reuse existing function)
    let state_info = get_rollback_state_info_with_changes(ctx, target_id, changes).await?;

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

/// Get rollback state information with pre-calculated changes
async fn get_rollback_state_info_with_changes(
    ctx: &OpsCtx,
    target_id: Uuid,
    changes: Vec<OpChange>,
) -> Result<StateInfo, Error> {
    let states = ctx.state.list_states_detailed().await?;
    let current_id = ctx.state.get_current_state_id().await?;

    let state =
        states
            .iter()
            .find(|s| s.state_id() == target_id)
            .ok_or(OpsError::StateNotFound {
                state_id: target_id,
            })?;

    let parent_id = state
        .parent_id
        .as_ref()
        .and_then(|p| uuid::Uuid::parse_str(p).ok());

    // Get actual package count for target state
    let package_count = get_state_package_count(ctx, &target_id).await?;

    Ok(StateInfo {
        id: target_id,
        parent: parent_id,
        timestamp: state.timestamp(),
        operation: state.operation.clone(),
        current: Some(current_id) == Some(target_id),
        package_count,
        total_size: 0, // TODO: Calculate actual size
        changes,       // Use pre-calculated changes
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
    let parent_packages = ctx.state.get_installed_packages_in_state(parent_id).await?;
    let child_packages = ctx.state.get_installed_packages_in_state(child_id).await?;

    let mut changes = Vec::new();

    // Convert to maps for easier comparison (name -> version)
    let parent_map: std::collections::HashMap<String, String> = parent_packages
        .iter()
        .map(|p| (p.name.clone(), p.version.clone()))
        .collect();
    let child_map: std::collections::HashMap<String, String> = child_packages
        .iter()
        .map(|p| (p.name.clone(), p.version.clone()))
        .collect();

    // Find packages that were added (in child but not parent)
    for (package_name, version) in &child_map {
        if !parent_map.contains_key(package_name) {
            changes.push(OpChange {
                change_type: ChangeType::Install,
                package: package_name.clone(),
                old_version: None,
                new_version: version.parse().ok(),
            });
        } else if let Some(parent_version) = parent_map.get(package_name) {
            // Check for version changes
            if version != parent_version {
                // For now, we'll just detect differences, not direction of change
                changes.push(OpChange {
                    change_type: ChangeType::Update,
                    package: package_name.clone(),
                    old_version: parent_version.parse().ok(),
                    new_version: version.parse().ok(),
                });
            }
        }
    }

    // Find packages that were removed (in parent but not child)
    for (package_name, version) in &parent_map {
        if !child_map.contains_key(package_name) {
            changes.push(OpChange {
                change_type: ChangeType::Remove,
                package: package_name.clone(),
                old_version: version.parse().ok(),
                new_version: None,
            });
        }
    }

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
    let timestamp_path = std::path::Path::new(sps2_config::fixed_paths::LAST_GC_TIMESTAMP);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    tokio::fs::write(timestamp_path, now.to_string())
        .await
        .map_err(|e| sps2_errors::Error::internal(format!("Failed to write GC timestamp: {e}")))?;

    Ok(())
}
