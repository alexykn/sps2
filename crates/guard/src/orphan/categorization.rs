//! Orphaned file categorization logic

use crate::types::OrphanedFileCategory;
use std::path::Path;

/// Categorize an orphaned file based on its path and characteristics
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub fn categorize_orphaned_file(path_str: &str, full_path: &Path) -> OrphanedFileCategory {
    // System files that should always be preserved
    if path_str.starts_with(".DS_Store")
        || path_str.starts_with("lost+found")
        || path_str.starts_with(".Spotlight-")
        || path_str.starts_with(".fseventsd")
        || path_str.starts_with(".Trashes")
    {
        return OrphanedFileCategory::System;
    }

    // Temporary files
    if path_str.ends_with(".tmp")
        || path_str.ends_with(".temp")
        || path_str.ends_with('~')
        || path_str.contains("/.cache/")
        || path_str.contains("/tmp/")
    {
        return OrphanedFileCategory::Temporary;
    }

    // User-created files (configs, data, etc)
    if path_str.ends_with(".conf")
        || path_str.ends_with(".config")
        || path_str.ends_with(".ini")
        || path_str.ends_with(".json")
        || path_str.ends_with(".yaml")
        || path_str.ends_with(".yml")
        || path_str.ends_with(".toml")
        || path_str.ends_with(".db")
        || path_str.ends_with(".sqlite")
        || path_str.contains("/data/")
        || path_str.contains("/config/")
        || path_str.contains("/var/")
    {
        return OrphanedFileCategory::UserCreated;
    }

    // Check if it might be a leftover from a previous package version
    // by looking at common binary/library extensions
    if path_str.ends_with(".so")
        || path_str.ends_with(".dylib")
        || path_str.ends_with(".a")
        || path_str.contains("/bin/")
        || path_str.contains("/lib/")
        || path_str.contains("/share/")
    {
        // Additional check: if file is executable, likely leftover
        if let Ok(metadata) = std::fs::metadata(full_path) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 != 0 {
                    return OrphanedFileCategory::Leftover;
                }
            }
        }
        return OrphanedFileCategory::Leftover;
    }

    // Default to unknown for further investigation
    OrphanedFileCategory::Unknown
}
