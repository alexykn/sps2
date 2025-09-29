//! Package management operations for atomic installation
//!
//! This module handles:
//! - Carrying forward packages from parent states
//! - Syncing staging slots with parent state
//! - Installing packages to staging
//! - Removing packages from staging

use crate::atomic::fs;
use crate::atomic::transition::StateTransition;
use crate::{InstallResult, PreparedPackage};
use sps2_errors::{Error, InstallError};
use sps2_hash::Hash;
use sps2_resolver::{PackageId, ResolvedNode};
use sps2_state::{file_queries_runtime, PackageRef, StateManager};
use sps2_store::PackageStore;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use uuid::Uuid;

/// Carry forward packages from parent state, excluding specified packages
///
/// This function registers package references for packages that are unchanged
/// between the parent state and the new staging state.
pub(super) fn carry_forward_packages(
    transition: &mut StateTransition,
    parent_packages: &[sps2_state::models::Package],
    exclude_names: &HashSet<String>,
) {
    if transition.parent_id.is_none() {
        return;
    }

    for pkg in parent_packages {
        if exclude_names.contains(&pkg.name) {
            continue;
        }

        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: PackageId::new(pkg.name.clone(), pkg.version()),
            hash: pkg.hash.clone(),
            size: pkg.size,
        };
        transition.package_refs.push(package_ref);
    }
}

/// Sync staging slot with parent state
///
/// This ensures the staging slot mirrors the parent state by:
/// - Removing packages that are no longer present
/// - Linking packages that are missing from the slot
///
/// # Errors
///
/// Returns an error if filesystem operations or state manager operations fail.
pub(super) async fn sync_slot_with_parent(
    state_manager: &StateManager,
    store: &PackageStore,
    transition: &mut StateTransition,
    parent_packages: &[sps2_state::models::Package],
) -> Result<(), Error> {
    let Some(parent_state) = transition.parent_id else {
        // No prior state to mirror; ensure slot metadata is cleared.
        state_manager
            .set_slot_state(transition.staging_slot, None)
            .await?;
        return Ok(());
    };

    let slot_state = state_manager.slot_state(transition.staging_slot).await;

    if slot_state == Some(parent_state) {
        return Ok(());
    }

    let slot_packages = if let Some(slot_state_id) = slot_state {
        state_manager
            .get_installed_packages_in_state(&slot_state_id)
            .await?
    } else {
        Vec::new()
    };

    let parent_keys: HashSet<String> = parent_packages
        .iter()
        .map(|pkg| format!("{}::{}", pkg.name, pkg.version))
        .collect();

    let slot_map: HashMap<String, sps2_state::models::Package> = slot_packages
        .into_iter()
        .map(|pkg| (format!("{}::{}", pkg.name, pkg.version), pkg))
        .collect();

    // Remove packages that should no longer be present in the slot
    for (key, pkg) in &slot_map {
        if !parent_keys.contains(key) {
            remove_package_from_staging(state_manager, transition, pkg).await?;
        }
    }

    // Link packages that are present in the parent state but missing from the slot
    for pkg in parent_packages {
        let key = format!("{}::{}", pkg.name, pkg.version);
        if slot_map.contains_key(&key) {
            continue;
        }

        let hash = Hash::from_hex(&pkg.hash).map_err(|e| {
            Error::from(InstallError::AtomicOperationFailed {
                message: format!(
                    "invalid package hash for {}-{} during slot sync: {e}",
                    pkg.name, pkg.version
                ),
            })
        })?;

        let store_path = store.package_path(&hash);
        let package_id = PackageId::new(pkg.name.clone(), pkg.version());
        link_package_to_staging(transition, &store_path, &package_id, false).await?;
    }

    state_manager
        .set_slot_state(transition.staging_slot, Some(parent_state))
        .await?;

    Ok(())
}

/// Install a single package to staging directory
///
/// This function:
/// - Validates prepared package data
/// - Handles package upgrades by removing old versions
/// - Ensures store references exist
/// - Links package files to staging
/// - Registers package references
///
/// # Errors
///
/// Returns an error if package data is missing, filesystem operations fail,
/// or state manager operations fail.
pub(super) async fn install_package_to_staging(
    state_manager: &StateManager,
    transition: &mut StateTransition,
    package_id: &PackageId,
    node: &ResolvedNode,
    prepared_package: Option<&PreparedPackage>,
    prior_package: Option<&sps2_state::models::Package>,
    result: &mut InstallResult,
) -> Result<(), Error> {
    // Install the package files (both Download and Local actions are handled identically)
    let action_name = match &node.action {
        sps2_resolver::NodeAction::Download => "downloaded",
        sps2_resolver::NodeAction::Local => "local",
    };

    let prepared = prepared_package.ok_or_else(|| {
        InstallError::AtomicOperationFailed {
            message: format!(
                "Missing prepared package data for {} package {}-{}. This indicates a bug in ParallelExecutor.",
                action_name, package_id.name, package_id.version
            ),
        }
    })?;

    let hash = &prepared.hash;
    let store_path = &prepared.store_path;
    let size = prepared.size;
    let store_hash_hex = hash.to_hex();
    let package_hash_hex = prepared.package_hash.as_ref().map(sps2_hash::Hash::to_hex);

    let mut was_present = false;
    let mut version_changed = false;
    if let Some(existing) = prior_package {
        was_present = true;
        let existing_version = existing.version();
        if existing_version != package_id.version {
            version_changed = true;
            remove_package_from_staging(state_manager, transition, existing).await?;
        }
    }

    // Load package from the prepared store path
    let _stored_package = sps2_store::StoredPackage::load(store_path).await?;

    // Ensure store_refs entry exists before adding to package_map
    let size_i64 = i64::try_from(size).map_err(|_| {
        Error::from(InstallError::AtomicOperationFailed {
            message: format!(
                "Package size {} exceeds maximum supported size for {}-{}",
                size, package_id.name, package_id.version
            ),
        })
    })?;
    state_manager
        .ensure_store_ref(&store_hash_hex, size_i64)
        .await?;

    // Ensure package is in package_map for future lookups
    state_manager
        .add_package_map(
            &package_id.name,
            &package_id.version.to_string(),
            &store_hash_hex,
            package_hash_hex.as_deref(),
        )
        .await?;

    // Link package files to staging
    let (_, file_hashes) =
        link_package_to_staging(transition, store_path, package_id, true).await?;

    // Store file hashes if we got them
    if let Some(hashes) = file_hashes {
        transition
            .pending_file_hashes
            .push((package_id.clone(), hashes));
    }

    // Add the package reference
    let package_ref = PackageRef {
        state_id: transition.staging_id,
        package_id: package_id.clone(),
        hash: store_hash_hex.clone(),
        size: size_i64,
    };
    transition.package_refs.push(package_ref);

    if was_present && version_changed {
        result.add_updated(package_id.clone());
    } else {
        result.add_installed(package_id.clone());
    }
    Ok(())
}

/// Link package from store to staging directory
///
/// This is a wrapper around the `fs::link_package_to_staging` function that
/// maintains backward compatibility with the existing installer code.
///
/// # Errors
///
/// Returns an error if the package cannot be loaded or linked.
async fn link_package_to_staging(
    transition: &mut StateTransition,
    store_path: &Path,
    package_id: &PackageId,
    record_hashes: bool,
) -> Result<(bool, Option<Vec<sps2_hash::FileHashResult>>), Error> {
    fs::link_package_to_staging(transition, store_path, package_id, record_hashes).await
}

/// Remove package files from staging directory
///
/// This function:
/// - Queries the database for all files belonging to the package
/// - Removes files in safe order (symlinks, regular files, directories)
/// - Cleans up Python runtime artifacts if applicable
///
/// # Errors
///
/// Returns an error if database queries fail or filesystem operations fail.
pub(super) async fn remove_package_from_staging(
    state_manager: &StateManager,
    transition: &mut StateTransition,
    package: &sps2_state::models::Package,
) -> Result<(), Error> {
    // Get all files belonging to this package from the database
    let state_id =
        Uuid::parse_str(&package.state_id).map_err(|e| InstallError::AtomicOperationFailed {
            message: format!(
                "failed to parse associated state ID for package {}: {e}",
                package.name
            ),
        })?;

    let mut tx = state_manager.begin_transaction().await?;
    let entries = file_queries_runtime::get_package_file_entries_by_name(
        &mut tx,
        &state_id,
        &package.name,
        &package.version,
    )
    .await?;
    tx.commit().await?;

    let file_paths: Vec<String> = entries
        .into_iter()
        .map(|entry| entry.relative_path)
        .collect();

    // Detect if this is a Python package for later cleanup
    let python_package_dir = fs::detect_python_package_directory(&file_paths);

    // Remove all tracked files using the fs module
    fs::remove_tracked_entries(transition, &file_paths).await?;

    // After removing all tracked files, clean up any remaining Python runtime artifacts
    if let Some(python_dir) = python_package_dir {
        fs::cleanup_python_runtime_artifacts(transition, &python_dir).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs as afs;

    async fn mk_env() -> (TempDir, StateManager, PackageStore) {
        let td = TempDir::new().expect("td");
        let state = StateManager::new(td.path()).await.expect("state");
        let store_base = td.path().join("store");
        afs::create_dir_all(&store_base).await.unwrap();
        let store = PackageStore::new(store_base);
        (td, state, store)
    }

    #[tokio::test]
    async fn test_carry_forward_excludes_specified_packages() {
        let (_td, state, _store) = mk_env().await;
        let mut transition = StateTransition::new(&state, "test".to_string())
            .await
            .unwrap();

        let parent_packages = vec![
            sps2_state::models::Package {
                id: 0,
                state_id: transition.staging_id.to_string(),
                name: "pkg-a".to_string(),
                version: "1.0.0".to_string(),
                hash: "abc123".to_string(),
                size: 1000,
                installed_at: chrono::Utc::now().timestamp(),
                venv_path: None,
            },
            sps2_state::models::Package {
                id: 0,
                state_id: transition.staging_id.to_string(),
                name: "pkg-b".to_string(),
                version: "2.0.0".to_string(),
                hash: "def456".to_string(),
                size: 2000,
                installed_at: chrono::Utc::now().timestamp(),
                venv_path: None,
            },
        ];

        let mut exclude_names = HashSet::new();
        exclude_names.insert("pkg-a".to_string());

        carry_forward_packages(&mut transition, &parent_packages, &exclude_names);

        // Should only carry forward pkg-b
        assert_eq!(transition.package_refs.len(), 1);
        assert_eq!(transition.package_refs[0].package_id.name, "pkg-b");
    }

    #[tokio::test]
    async fn test_carry_forward_all_when_no_exclusions() {
        let (_td, state, _store) = mk_env().await;
        let mut transition = StateTransition::new(&state, "test".to_string())
            .await
            .unwrap();

        let parent_packages = vec![
            sps2_state::models::Package {
                id: 0,
                state_id: transition.staging_id.to_string(),
                name: "pkg-a".to_string(),
                version: "1.0.0".to_string(),
                hash: "abc123".to_string(),
                size: 1000,
                installed_at: chrono::Utc::now().timestamp(),
                venv_path: None,
            },
            sps2_state::models::Package {
                id: 0,
                state_id: transition.staging_id.to_string(),
                name: "pkg-b".to_string(),
                version: "2.0.0".to_string(),
                hash: "def456".to_string(),
                size: 2000,
                installed_at: chrono::Utc::now().timestamp(),
                venv_path: None,
            },
        ];

        let exclude_names = HashSet::new();

        carry_forward_packages(&mut transition, &parent_packages, &exclude_names);

        // Should carry forward both packages
        assert_eq!(transition.package_refs.len(), 2);
    }
}
