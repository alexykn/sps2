//! Orphaned file detection logic

use crate::orphan::categorization::categorize_orphaned_file;
use crate::types::{Discrepancy, OrphanedFileCategory, VerificationScope};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Find orphaned files (files in live but not tracked in DB)
pub fn find_orphaned_files(
    live_path: &Path,
    tracked_files: &HashSet<PathBuf>,
    discrepancies: &mut Vec<Discrepancy>,
) {
    use walkdir::WalkDir;

    // Walk the live directory
    for entry in WalkDir::new(live_path)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        if let Ok(relative) = path.strip_prefix(live_path) {
            // Skip the root directory itself
            if relative.as_os_str().is_empty() {
                continue;
            }

            let relative_path = relative.to_path_buf();

            // Check if this file is tracked
            if !tracked_files.contains(&relative_path) {
                let path_str = relative_path.to_string_lossy();

                // Categorize the orphaned file
                let category = categorize_orphaned_file(&path_str, path);

                // Skip system files that should always be preserved
                if matches!(category, OrphanedFileCategory::System) {
                    continue;
                }

                discrepancies.push(Discrepancy::OrphanedFile {
                    file_path: path_str.to_string(),
                    category,
                });
            }
        }
    }
}

/// Find orphaned files limited to specific directories based on scope
pub fn find_orphaned_files_scoped(
    scope: &VerificationScope,
    live_path: &Path,
    tracked_files: &HashSet<PathBuf>,
    discrepancies: &mut Vec<Discrepancy>,
) -> Vec<PathBuf> {
    use walkdir::WalkDir;

    let directories_to_check = get_directories_to_check(scope, live_path);

    let mut checked_directories = Vec::new();

    for check_path in directories_to_check {
        checked_directories.push(check_path.clone());

        // Only walk if the directory exists
        if !check_path.exists() {
            continue;
        }

        // Walk the specified directory
        for entry in WalkDir::new(&check_path)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            let path = entry.path();
            if let Ok(relative) = path.strip_prefix(live_path) {
                // Skip the root directory itself
                if relative.as_os_str().is_empty() {
                    continue;
                }

                let relative_path = relative.to_path_buf();

                // Check if this file is tracked
                if !tracked_files.contains(&relative_path) {
                    let path_str = relative_path.to_string_lossy();

                    // Categorize the orphaned file
                    let category = categorize_orphaned_file(&path_str, path);

                    // Skip system files that should always be preserved
                    if matches!(category, OrphanedFileCategory::System) {
                        continue;
                    }

                    discrepancies.push(Discrepancy::OrphanedFile {
                        file_path: path_str.to_string(),
                        category,
                    });
                }
            }
        }
    }

    checked_directories
}

/// Get directories to check for orphaned files based on verification scope
pub fn get_directories_to_check(scope: &VerificationScope, live_path: &Path) -> Vec<PathBuf> {
    match scope {
        VerificationScope::Full => {
            // Check entire live directory (current behavior)
            vec![live_path.to_path_buf()]
        }
        VerificationScope::Directory { path } => {
            // Check specific directory
            vec![live_path.join(path)]
        }
        VerificationScope::Directories { paths } => {
            // Check multiple directories
            paths.iter().map(|p| live_path.join(p)).collect()
        }
        VerificationScope::Mixed {
            directories,
            packages: _,
        } => {
            // Check specified directories
            directories.iter().map(|p| live_path.join(p)).collect()
        }
        VerificationScope::Package { .. } | VerificationScope::Packages { .. } => {
            // For package-only scopes, don't check for orphaned files unless explicitly requested
            // This is a performance optimization - users verifying specific packages
            // probably don't care about orphaned files elsewhere
            vec![]
        }
    }
}
