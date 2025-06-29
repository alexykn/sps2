//! Scoped verification helpers

use crate::types::VerificationScope;
use sps2_errors::Error;
use sps2_state::{queries, StateManager};
use std::collections::HashSet;
use uuid::Uuid;

/// Get packages to verify based on verification scope
///
/// Returns (packages_to_verify, total_packages, total_files_in_packages_to_verify)
pub async fn get_packages_for_scope(
    state_manager: &StateManager,
    state_id: &Uuid,
    scope: &VerificationScope,
) -> Result<(Vec<sps2_state::models::Package>, usize, usize), Error> {
    let mut tx = state_manager.begin_transaction().await?;

    match scope {
        VerificationScope::Full => {
            // Get all packages (current behavior)
            let all_packages = queries::get_state_packages(&mut tx, state_id).await?;
            tx.commit().await?;
            let total_files = count_total_files(state_manager, state_id, &all_packages).await?;
            Ok((all_packages.clone(), all_packages.len(), total_files))
        }
        VerificationScope::Package { name, version } => {
            // Get specific package
            let all_packages = queries::get_state_packages(&mut tx, state_id).await?;
            let specific_package = all_packages
                .into_iter()
                .filter(|p| p.name == *name && p.version == *version)
                .collect::<Vec<_>>();

            tx.commit().await?;
            let total_files = if specific_package.is_empty() {
                0
            } else {
                count_total_files(state_manager, state_id, &specific_package).await?
            };
            Ok((specific_package, 1, total_files)) // total_packages = 1 for single package
        }
        VerificationScope::Packages { packages } => {
            // Get multiple specific packages
            let all_packages = queries::get_state_packages(&mut tx, state_id).await?;
            let package_set: HashSet<(String, String)> = packages.iter().cloned().collect();
            let filtered_packages = all_packages
                .into_iter()
                .filter(|p| package_set.contains(&(p.name.clone(), p.version.clone())))
                .collect::<Vec<_>>();

            tx.commit().await?;
            let total_files = if filtered_packages.is_empty() {
                0
            } else {
                count_total_files(state_manager, state_id, &filtered_packages).await?
            };

            Ok((filtered_packages, packages.len(), total_files))
        }
        VerificationScope::Directory { path: _ } => {
            // For directory-only scope, we still need to get all packages to check which files
            // belong to the directory, but we'll filter during verification
            let all_packages = queries::get_state_packages(&mut tx, state_id).await?;
            tx.commit().await?;
            let total_files = count_total_files(state_manager, state_id, &all_packages).await?;
            Ok((all_packages.clone(), all_packages.len(), total_files))
        }
        VerificationScope::Directories { paths: _ } => {
            // Similar to single directory
            let all_packages = queries::get_state_packages(&mut tx, state_id).await?;
            tx.commit().await?;
            let total_files = count_total_files(state_manager, state_id, &all_packages).await?;
            Ok((all_packages.clone(), all_packages.len(), total_files))
        }
        VerificationScope::Mixed {
            packages,
            directories: _,
        } => {
            // Get specified packages, but we'll still need directory filtering during verification
            let all_packages = queries::get_state_packages(&mut tx, state_id).await?;
            let package_set: HashSet<(String, String)> = packages.iter().cloned().collect();
            let filtered_packages = all_packages
                .into_iter()
                .filter(|p| package_set.contains(&(p.name.clone(), p.version.clone())))
                .collect::<Vec<_>>();

            tx.commit().await?;
            let total_files = if filtered_packages.is_empty() {
                0
            } else {
                count_total_files(state_manager, state_id, &filtered_packages).await?
            };

            Ok((filtered_packages, packages.len(), total_files))
        }
    }
}

/// Count total files for a set of packages
pub async fn count_total_files(
    state_manager: &StateManager,
    state_id: &Uuid,
    packages: &[sps2_state::models::Package],
) -> Result<usize, Error> {
    let mut total_files = 0;
    for package in packages {
        let mut tx = state_manager.begin_transaction().await?;
        let file_paths =
            queries::get_package_files(&mut tx, state_id, &package.name, &package.version).await?;
        tx.commit().await?;
        total_files += file_paths.len();
    }
    Ok(total_files)
}

/// Check if a file path should be verified based on the verification scope
pub fn should_verify_file(file_path: &std::path::Path, scope: &VerificationScope) -> bool {
    match scope {
        VerificationScope::Full => true,
        VerificationScope::Package { .. } | VerificationScope::Packages { .. } => true,
        VerificationScope::Directory { path } => file_path.starts_with(path),
        VerificationScope::Directories { paths } => {
            paths.iter().any(|dir| file_path.starts_with(dir))
        }
        VerificationScope::Mixed { directories, .. } => {
            if directories.is_empty() {
                true
            } else {
                directories.iter().any(|dir| file_path.starts_with(dir))
            }
        }
    }
}
