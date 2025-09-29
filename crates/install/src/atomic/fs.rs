//! Filesystem operations for atomic installation

use crate::atomic::transition::StateTransition;
use sps2_errors::{Error, InstallError};
use sps2_events::{AppEvent, EventEmitter, GeneralEvent};
use sps2_hash::FileHashResult;
use sps2_resolver::PackageId;
use sps2_store::StoredPackage;
use std::path::Path;

/// Link package from store to staging directory
///
/// Returns (`had_file_hashes`, `file_hashes`) where `file_hashes` is Some only if `record_hashes` is true
pub(super) async fn link_package_to_staging(
    transition: &mut StateTransition,
    store_path: &Path,
    package_id: &PackageId,
    record_hashes: bool,
) -> Result<(bool, Option<Vec<FileHashResult>>), Error> {
    let staging_prefix = &transition.slot_path;

    // Load the stored package
    let stored_package = StoredPackage::load(store_path).await?;

    // Link files from store to staging
    if let Some(sender) = &transition.event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!("Linking package {} to staging", package_id.name),
            context: std::collections::HashMap::new(),
        }));
    }

    stored_package.link_to(staging_prefix).await?;

    let mut had_file_hashes = false;
    let mut linked_entry_count = 0usize;
    let mut file_hashes_result = None;

    // Collect file paths for database tracking AND store file hash info
    if let Some(file_hashes) = stored_package.file_hashes() {
        had_file_hashes = true;
        linked_entry_count = file_hashes.len();
        // Store the file hash information for later use when we have package IDs
        if record_hashes {
            file_hashes_result = Some(file_hashes.to_vec());
        }
    }

    // Debug what was linked
    if let Some(sender) = &transition.event_sender {
        sender.emit(AppEvent::General(GeneralEvent::DebugLog {
            message: format!(
                "Linked {} files/directories for package {}",
                linked_entry_count, package_id.name
            ),
            context: std::collections::HashMap::new(),
        }));
    }

    Ok((had_file_hashes, file_hashes_result))
}

/// Remove tracked package entries from staging directory
///
/// Takes a transition and a list of file paths, removes them in safe order:
/// symlinks first, then regular files, then directories (deepest first)
pub(super) async fn remove_tracked_entries(
    transition: &StateTransition,
    file_paths: &[String],
) -> Result<(), Error> {
    // Group files by type for proper removal order
    let mut symlinks = Vec::new();
    let mut regular_files = Vec::new();
    let mut directories = Vec::new();

    for file_path in file_paths {
        let staging_file = transition.slot_path.join(file_path);

        if staging_file.exists() {
            // Check if it's a symlink
            let metadata = tokio::fs::symlink_metadata(&staging_file).await?;
            if metadata.is_symlink() {
                symlinks.push(file_path.clone());
            } else if staging_file.is_dir() {
                directories.push(file_path.clone());
            } else {
                regular_files.push(file_path.clone());
            }
        }
    }

    // Remove in order: symlinks first, then files, then directories
    // This ensures we don't try to remove non-empty directories

    // 1. Remove symlinks
    for file_path in symlinks {
        let staging_file = transition.slot_path.join(&file_path);
        if staging_file.exists() {
            tokio::fs::remove_file(&staging_file).await.map_err(|e| {
                InstallError::FilesystemError {
                    operation: "remove_symlink".to_string(),
                    path: staging_file.display().to_string(),
                    message: e.to_string(),
                }
            })?;
        }
    }

    // 2. Remove regular files
    for file_path in regular_files {
        let staging_file = transition.slot_path.join(&file_path);
        if staging_file.exists() {
            tokio::fs::remove_file(&staging_file).await.map_err(|e| {
                InstallError::FilesystemError {
                    operation: "remove_file".to_string(),
                    path: staging_file.display().to_string(),
                    message: e.to_string(),
                }
            })?;
        }
    }

    // 3. Remove directories in reverse order (deepest first)
    directories.sort_by(|a, b| b.cmp(a)); // Reverse lexicographic order
    for file_path in directories {
        let staging_file = transition.slot_path.join(&file_path);
        if staging_file.exists() {
            // Try to remove directory if it's empty
            if let Ok(mut entries) = tokio::fs::read_dir(&staging_file).await {
                if entries.next_entry().await?.is_none() {
                    tokio::fs::remove_dir(&staging_file).await.map_err(|e| {
                        InstallError::FilesystemError {
                            operation: "remove_dir".to_string(),
                            path: staging_file.display().to_string(),
                            message: e.to_string(),
                        }
                    })?;
                }
            }
        }
    }

    Ok(())
}

/// Detect if this is a Python package and return the directory to remove
///
/// Python packages are isolated in `/opt/pm/live/python/<package_name>/` directories.
/// This method examines file paths to find the Python package directory.
pub(super) fn detect_python_package_directory(file_paths: &[String]) -> Option<String> {
    for file_path in file_paths {
        // Look for files under python/ directory structure
        if let Some(stripped) = file_path.strip_prefix("python/") {
            // Extract the package directory (e.g., "ansible/" from "python/ansible/lib/...")
            if let Some(slash_pos) = stripped.find('/') {
                let package_dir = format!("python/{}", &stripped[..slash_pos]);
                return Some(package_dir);
            } else if !stripped.is_empty() {
                // Handle case where the path is just "python/package_name"
                let package_dir = format!("python/{stripped}");
                return Some(package_dir);
            }
        }
    }
    None
}

/// Clean up remaining Python runtime artifacts
///
/// After removing tracked files, this removes any remaining runtime artifacts
/// (e.g., __pycache__, .pyc files) that weren't explicitly tracked
pub(super) async fn cleanup_python_runtime_artifacts(
    transition: &StateTransition,
    python_dir: &str,
) -> Result<(), Error> {
    let python_staging_dir = transition.slot_path.join(python_dir);

    if python_staging_dir.exists() {
        // Check if directory still has content (runtime artifacts)
        if let Ok(mut entries) = tokio::fs::read_dir(&python_staging_dir).await {
            if entries.next_entry().await?.is_some() {
                // Directory is not empty, remove remaining runtime artifacts
                tokio::fs::remove_dir_all(&python_staging_dir)
                    .await
                    .map_err(|e| InstallError::FilesystemError {
                        operation: "cleanup_python_runtime_artifacts".to_string(),
                        path: python_staging_dir.display().to_string(),
                        message: e.to_string(),
                    })?;
            }
        }
    }

    Ok(())
}
