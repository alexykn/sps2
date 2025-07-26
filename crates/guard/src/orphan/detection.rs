//! Orphaned file detection logic

use crate::orphan::categorization::categorize_orphaned_file;
use crate::types::{Discrepancy, OrphanedFileCategory};
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

                // Skip files that should be ignored during verification
                if matches!(
                    category,
                    OrphanedFileCategory::System | OrphanedFileCategory::RuntimeGenerated
                ) {
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
