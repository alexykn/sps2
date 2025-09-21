//! System Cleanup and State Management Operations

use crate::{ChangeType, OpChange, OpsCtx, StateInfo};
use sps2_errors::{Error, OpsError};
use sps2_events::{AppEvent, EventEmitter, GeneralEvent, PackageEvent, StateEvent};
use std::time::Instant;

async fn compute_kept_states(
    ctx: &OpsCtx,
    keep_count: usize,
    keep_days: i64,
) -> Result<std::collections::HashSet<Uuid>, Error> {
    let now = chrono::Utc::now().timestamp();
    let states = ctx.state.list_states_detailed().await?;
    let mut kept: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    for id in states.iter().take(keep_count) {
        kept.insert(id.state_id());
    }
    if keep_days > 0 {
        let cutoff = now - (keep_days * 86_400);
        for st in &states {
            if st.created_at >= cutoff {
                kept.insert(st.state_id());
            } else {
                break;
            }
        }
    }
    kept.insert(ctx.state.get_current_state_id().await?);
    Ok(kept)
}

async fn collect_required_hashes(
    ctx: &OpsCtx,
    kept: &std::collections::HashSet<Uuid>,
) -> Result<
    (
        std::collections::HashSet<String>,
        std::collections::HashSet<String>,
    ),
    Error,
> {
    let mut required_pkg_hashes: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for st in kept {
        let pkgs = ctx.state.get_installed_packages_in_state(st).await?;
        for p in pkgs {
            required_pkg_hashes.insert(p.hash);
        }
    }

    let mut required_file_hashes: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    {
        let mut tx = ctx.state.begin_transaction().await?;
        for st in kept {
            let packages = sps2_state::queries::get_state_packages(&mut tx, st).await?;
            for pkg in &packages {
                let entries =
                    sps2_state::file_queries_runtime::get_package_file_entries(&mut tx, pkg.id)
                        .await?;
                for e in entries {
                    required_file_hashes.insert(e.file_hash);
                }
            }
        }
        tx.commit().await?;
    }
    Ok((required_pkg_hashes, required_file_hashes))
}

struct LastRefData {
    pkg_last_ref: std::collections::HashMap<String, i64>,
    obj_last_ref: std::collections::HashMap<String, i64>,
    store_refs: Vec<sps2_state::StoreRef>,
    file_objs: Vec<sps2_state::FileObject>,
}

async fn collect_last_ref_and_inventory(ctx: &OpsCtx) -> Result<LastRefData, Error> {
    let mut tx = ctx.state.begin_transaction().await?;
    let store_refs = sps2_state::queries::get_all_store_refs(&mut tx).await?;
    let pkg_last_ref = sps2_state::queries::get_package_last_ref_map(&mut tx).await?;
    let obj_last_ref = sps2_state::file_queries_runtime::get_file_last_ref_map(&mut tx).await?;
    let file_objs = sps2_state::file_queries_runtime::get_all_file_objects(&mut tx).await?;
    tx.commit().await?;
    Ok(LastRefData {
        pkg_last_ref,
        obj_last_ref,
        store_refs,
        file_objs,
    })
}

async fn evict_packages(
    ctx: &OpsCtx,
    last: &LastRefData,
    required_pkg_hashes: &std::collections::HashSet<String>,
    pkg_grace_secs: i64,
    now: i64,
    dry_run: bool,
) -> Result<(usize, i64), Error> {
    let mut count = 0usize;
    let mut bytes = 0i64;
    for sr in &last.store_refs {
        if required_pkg_hashes.contains(&sr.hash) {
            continue;
        }
        let last_ref = *last.pkg_last_ref.get(&sr.hash).unwrap_or(&0);
        if last_ref == 0 || (now - last_ref) >= pkg_grace_secs {
            let hash = sps2_hash::Hash::from_hex(&sr.hash).map_err(|e| {
                sps2_errors::Error::internal(format!("invalid hash {}: {e}", sr.hash))
            })?;
            if !dry_run {
                let _ = ctx.store.remove_package(&hash).await;
                let mut tx = ctx.state.begin_transaction().await?;
                sps2_state::queries::insert_package_eviction(
                    &mut tx,
                    &sr.hash,
                    sr.size,
                    Some("policy"),
                )
                .await?;
                tx.commit().await?;
            }
            count += 1;
            bytes += sr.size;
        }
    }
    Ok((count, bytes))
}

async fn evict_objects(
    ctx: &OpsCtx,
    last: &LastRefData,
    required_file_hashes: &std::collections::HashSet<String>,
    obj_grace_secs: i64,
    now: i64,
    dry_run: bool,
) -> Result<(usize, i64), Error> {
    let mut count = 0usize;
    let mut bytes = 0i64;
    for fo in &last.file_objs {
        if required_file_hashes.contains(&fo.hash) {
            continue;
        }
        let last_ref = *last.obj_last_ref.get(&fo.hash).unwrap_or(&0);
        if last_ref == 0 || (now - last_ref) >= obj_grace_secs {
            let fh = sps2_hash::Hash::from_hex(&fo.hash).map_err(|e| {
                sps2_errors::Error::internal(format!("invalid file hash {}: {e}", fo.hash))
            })?;
            if !dry_run {
                let _ = ctx.store.file_store().remove_file(&fh).await;
                let mut tx = ctx.state.begin_transaction().await?;
                sps2_state::queries::insert_file_object_eviction(
                    &mut tx,
                    &fo.hash,
                    fo.size,
                    Some("policy"),
                )
                .await?;
                tx.commit().await?;
            }
            count += 1;
            bytes += fo.size;
        }
    }
    Ok((count, bytes))
}
use uuid::Uuid;

/// Clean up orphaned packages and old states
///
/// # Errors
///
/// Returns an error if cleanup operation fails.
pub async fn cleanup(ctx: &OpsCtx) -> Result<String, Error> {
    let start = Instant::now();
    ctx.emit(AppEvent::Package(PackageEvent::CleanupStarting));

    // Legacy snapshots and orphaned stagings
    let cleanup_result = ctx
        .state
        .cleanup(
            ctx.config.state.retention_count,
            ctx.config.state.retention_days,
        )
        .await?;

    // CAS policy cleanup
    let cas_cfg = &ctx.config.cas;
    let keep_count = cas_cfg.keep_states_count;
    let keep_days = i64::from(cas_cfg.keep_days);
    let pkg_grace_secs = i64::from(cas_cfg.package_grace_days) * 86_400;
    let obj_grace_secs = i64::from(cas_cfg.object_grace_days) * 86_400;
    let now = chrono::Utc::now().timestamp();

    let kept = compute_kept_states(ctx, keep_count, keep_days).await?;
    let (required_pkg_hashes, required_file_hashes) = collect_required_hashes(ctx, &kept).await?;
    let last = collect_last_ref_and_inventory(ctx).await?;

    let (packages_evicted, pkg_space_freed) = evict_packages(
        ctx,
        &last,
        &required_pkg_hashes,
        pkg_grace_secs,
        now,
        cas_cfg.dry_run,
    )
    .await?;
    let (objects_evicted, obj_space_freed) = evict_objects(
        ctx,
        &last,
        &required_file_hashes,
        obj_grace_secs,
        now,
        cas_cfg.dry_run,
    )
    .await?;

    let duration = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let message = if cas_cfg.dry_run {
        format!(
            "Dry-run: would prune {} states, remove {} dirs, {} packages ({} bytes), {} objects ({} bytes)",
            cleanup_result.states_pruned,
            cleanup_result.states_removed,
            packages_evicted,
            pkg_space_freed,
            objects_evicted,
            obj_space_freed
        )
    } else {
        format!(
            "Pruned {} states, cleaned {} dirs, removed {} packages ({} bytes), {} objects ({} bytes)",
            cleanup_result.states_pruned,
            cleanup_result.states_removed,
            packages_evicted,
            pkg_space_freed,
            objects_evicted,
            obj_space_freed
        )
    };

    ctx.emit(AppEvent::Package(PackageEvent::CleanupCompleted {
        states_removed: cleanup_result.states_removed,
        packages_removed: packages_evicted,
        duration_ms: duration,
    }));

    if let Err(e) = update_gc_timestamp().await {
        use sps2_events::events::GeneralEvent;
        ctx.emit(AppEvent::General(GeneralEvent::warning_with_context(
            "Failed to update GC timestamp",
            e.to_string(),
        )));
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

    let current_before = ctx.state.get_current_state_id().await?;

    // If no target specified, rollback to previous state
    let target_id = if let Some(id) = target_state {
        id
    } else {
        ctx.state
            .get_parent_state_id(&current_before)
            .await?
            .ok_or(OpsError::NoPreviousState)?
    };

    ctx.emit(AppEvent::State(StateEvent::RollbackStarted {
        from: current_before,
        to: target_id,
    }));

    // Verify target state exists in database
    if !ctx.state.state_exists(&target_id).await? {
        return Err(OpsError::StateNotFound {
            state_id: target_id,
        }
        .into());
    }

    // Filesystem snapshot presence is no longer required; rollback reconstructs incrementally.

    // Calculate changes BEFORE rollback (current -> target)
    let rollback_changes = calculate_state_changes(ctx, &current_before, &target_id).await?;

    // Perform rollback using atomic installer
    let mut atomic_installer =
        sps2_install::AtomicInstaller::new(ctx.state.clone(), ctx.store.clone()).await?;

    // Move semantics: make target the active state without creating a new one
    atomic_installer.rollback_move_to_state(target_id).await?;

    // Get state information with pre-calculated changes
    let state_info = get_rollback_state_info_with_changes(ctx, target_id, rollback_changes).await?;

    ctx.tx.emit(AppEvent::State(StateEvent::RollbackCompleted {
        from: current_before,
        to: target_id,
        duration: Some(start.elapsed()),
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

    // Filesystem snapshot presence is no longer required for rollback preview.

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
pub async fn history(
    ctx: &OpsCtx,
    show_all: bool,
    verify: bool,
    limit_override: Option<usize>,
) -> Result<Vec<StateInfo>, Error> {
    let all_states = ctx.state.list_states_detailed().await?;
    let current_id = ctx.state.get_current_state_id().await?;

    if verify {
        // Deep verify across full DB history; cap by override or config (newest first)
        let limit = limit_override.unwrap_or(ctx.config.state.history_verify_limit);
        let mut out = Vec::new();
        for state in &all_states {
            let id = state.state_id();
            if is_state_available(ctx, &id).await? {
                let parent_id = state
                    .parent_id
                    .as_ref()
                    .and_then(|p| uuid::Uuid::parse_str(p).ok());
                let package_count = get_state_package_count(ctx, &id).await?;
                let changes = if let Some(parent_id) = parent_id {
                    calculate_state_changes(ctx, &parent_id, &id).await?
                } else {
                    get_initial_state_changes(ctx, &id).await?
                };
                out.push(StateInfo {
                    id,
                    parent: parent_id,
                    timestamp: state.timestamp(),
                    operation: state.operation.clone(),
                    current: Some(current_id) == Some(id),
                    package_count,
                    total_size: 0,
                    changes,
                });
                if out.len() >= limit {
                    break;
                }
            }
        }
        return Ok(out);
    }

    let mut state_infos = Vec::new();
    for state in &all_states {
        let id = state.state_id();
        // Default base history: show unpruned states; always include current
        if !(show_all || state.pruned_at.is_none() || Some(id) == Some(current_id)) {
            continue;
        }
        let parent_id = state
            .parent_id
            .as_ref()
            .and_then(|p| uuid::Uuid::parse_str(p).ok());
        let package_count = get_state_package_count(ctx, &id).await?;
        let changes = if let Some(parent_id) = parent_id {
            calculate_state_changes(ctx, &parent_id, &id).await?
        } else {
            get_initial_state_changes(ctx, &id).await?
        };
        state_infos.push(StateInfo {
            id,
            parent: parent_id,
            timestamp: state.timestamp(),
            operation: state.operation.clone(),
            current: Some(current_id) == Some(id),
            package_count,
            total_size: 0,
            changes,
        });
    }
    Ok(state_infos)
}

async fn is_state_available(ctx: &OpsCtx, state_id: &Uuid) -> Result<bool, Error> {
    // Check every package in the state
    let packages = ctx.state.get_installed_packages_in_state(state_id).await?;
    for pkg in packages {
        // Legacy path: package dir in store must exist
        let hash = sps2_hash::Hash::from_hex(&pkg.hash)
            .map_err(|e| sps2_errors::Error::internal(format!("invalid hash {}: {e}", pkg.hash)))?;
        if !ctx.store.has_package(&hash).await {
            return Ok(false);
        }

        // File-level path: ensure all referenced file objects exist
        let mut tx = ctx.state.begin_transaction().await?;
        let file_entries = sps2_state::file_queries_runtime::get_package_file_entries_by_name(
            &mut tx,
            state_id,
            &pkg.name,
            &pkg.version,
        )
        .await?;
        tx.commit().await?;

        if !file_entries.is_empty() {
            for entry in file_entries {
                let fh = sps2_hash::Hash::from_hex(&entry.file_hash).map_err(|e| {
                    sps2_errors::Error::internal(format!(
                        "invalid file hash {}: {e}",
                        entry.file_hash
                    ))
                })?;
                if !ctx.store.file_store().has_file(&fh).await {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
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
