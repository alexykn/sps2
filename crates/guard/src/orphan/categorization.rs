//! Orphaned file categorization logic

use crate::types::OrphanedFileCategory;
use std::path::Path;

/// Categorize an orphaned file based on its path and characteristics
#[allow(clippy::case_sensitive_file_extension_comparisons)] // macOS filesystem is case-sensitive
pub fn categorize_orphaned_file(path_str: &str, full_path: &Path) -> OrphanedFileCategory {
    // Python runtime-generated artifacts that should be ignored during verification
    if is_python_runtime_artifact(path_str, full_path) {
        return OrphanedFileCategory::RuntimeGenerated;
    }

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

/// Check if a file is a Python runtime-generated artifact that should be ignored
///
/// This matches the same patterns that PythonBytecodeCleanupPatcher removes during build.
/// We only consider files under python/ paths to avoid false positives.
fn is_python_runtime_artifact(path_str: &str, full_path: &Path) -> bool {
    // Only check files under python/ directory structure
    if !path_str.starts_with("python/") {
        return false;
    }

    let filename = full_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // 1. __pycache__ directories and their contents
    if path_str.contains("__pycache__") {
        return true;
    }

    // 2. Python bytecode files
    if let Some(ext) = full_path.extension().and_then(|e| e.to_str()) {
        if matches!(ext, "pyc" | "pyo") {
            return true;
        }

        // Complex bytecode patterns (.cpython-*.pyc, .pypy*.pyc, .opt-*.pyc)
        if ext.eq_ignore_ascii_case("pyc")
            && (filename.contains(".cpython-")
                || filename.contains(".pypy")
                || filename.contains(".opt-"))
        {
            return true;
        }
    }

    // 3. Python build artifacts and development directories (only in python/ paths)
    if matches!(
        filename,
        "build"
            | "dist"
            | ".eggs"
            | ".tox"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".ruff_cache"
            | "htmlcov"
            | ".DS_Store"
            | "Thumbs.db"
            | ".vscode"
            | ".idea"
    ) || filename.ends_with(".egg-info")
        || filename.starts_with("pip-build-env-")
        || filename.starts_with("pip-req-build-")
    {
        return true;
    }

    // 4. Pip metadata files (only in python/ paths)
    if matches!(filename, "INSTALLER" | "REQUESTED" | "direct_url.json") {
        return true;
    }

    false
}
