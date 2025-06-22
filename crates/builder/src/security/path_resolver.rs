//! Path resolution and normalization for security validation

use sps2_errors::{BuildError, Error};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// Normalize a path by resolving .., ., and symlinks
pub fn normalize_path(
    path: &Path,
    cache: &HashMap<PathBuf, PathBuf>,
    build_root: &Path,
) -> Result<PathBuf, Error> {
    // Check cache first
    if let Some(cached) = cache.get(path) {
        return Ok(cached.clone());
    }

    let mut normalized = PathBuf::new();
    let mut depth = 0;

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                normalized.push(component);
                depth = 0;
            }
            Component::CurDir => {
                // Skip .
            }
            Component::ParentDir => {
                if depth > 0 {
                    normalized.pop();
                    depth -= 1;
                } else if normalized.as_os_str().is_empty() {
                    // Relative path going up from current dir
                    normalized.push("..");
                } else {
                    // Trying to go above root
                    return Err(BuildError::PathTraversalAttempt {
                        path: path.display().to_string(),
                        reason: "Too many .. components".to_string(),
                    }
                    .into());
                }
            }
            Component::Normal(name) => {
                normalized.push(name);
                depth += 1;
            }
        }
    }

    // Resolve symlinks (with loop detection)
    resolve_symlinks_safe(&normalized, build_root)
}

/// Safely resolve symlinks with loop detection
fn resolve_symlinks_safe(path: &Path, build_root: &Path) -> Result<PathBuf, Error> {
    const MAX_SYMLINK_DEPTH: usize = 10;
    let mut visited = HashSet::new();
    let mut current = path.to_path_buf();
    let mut iterations = 0;

    while iterations < MAX_SYMLINK_DEPTH {
        if !visited.insert(current.clone()) {
            return Err(BuildError::SymlinkLoop {
                path: current.display().to_string(),
            }
            .into());
        }

        // Only resolve symlinks if the path exists
        if current.exists() {
            match std::fs::read_link(&current) {
                Ok(target) => {
                    current = if target.is_absolute() {
                        target
                    } else {
                        current
                            .parent()
                            .ok_or_else(|| BuildError::InvalidPath {
                                path: current.display().to_string(),
                                reason: "No parent directory".to_string(),
                            })?
                            .join(target)
                    };

                    // Normalize the new path
                    current = simple_normalize(&current)?;

                    // Check if symlink is trying to escape build root
                    if !is_path_safe(&current, build_root) {
                        return Err(BuildError::PathEscapeAttempt {
                            path: path.display().to_string(),
                            resolved: current.display().to_string(),
                            build_root: build_root.display().to_string(),
                        }
                        .into());
                    }

                    iterations += 1;
                }
                Err(_) => {
                    // Not a symlink, that's fine
                    break;
                }
            }
        } else {
            // Path doesn't exist yet, that's OK for write operations
            break;
        }
    }

    if iterations >= MAX_SYMLINK_DEPTH {
        return Err(BuildError::TooManySymlinks {
            path: path.display().to_string(),
        }
        .into());
    }

    Ok(current)
}

/// Simple path normalization without symlink resolution
fn simple_normalize(path: &Path) -> Result<PathBuf, Error> {
    let mut normalized = PathBuf::new();
    let mut depth = 0;

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                normalized.push(component);
                depth = 0;
            }
            Component::CurDir => {
                // Skip .
            }
            Component::ParentDir => {
                if depth > 0 {
                    normalized.pop();
                    depth -= 1;
                } else {
                    // Can't go up further
                    return Err(BuildError::PathTraversalAttempt {
                        path: path.display().to_string(),
                        reason: "Path goes above root".to_string(),
                    }
                    .into());
                }
            }
            Component::Normal(name) => {
                normalized.push(name);
                depth += 1;
            }
        }
    }

    Ok(normalized)
}

/// Check if a path is safe (doesn't escape build environment)
fn is_path_safe(path: &Path, build_root: &Path) -> bool {
    const SAFE_SYSTEM_PREFIXES: &[&str] = &[
        "/usr/include",
        "/usr/lib",
        "/usr/local/include",
        "/usr/local/lib",
        "/usr/bin",
        "/usr/local/bin",
        "/bin",
        "/opt/pm/live",
    ];

    // Allow paths within build root
    if path.starts_with(build_root) {
        return true;
    }

    // Allow certain system paths for reading

    SAFE_SYSTEM_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_simple_paths() {
        let cache = HashMap::new();
        let build_root = Path::new("/opt/pm/build/test");

        // Simple absolute path
        let result = normalize_path(Path::new("/opt/pm/build/test/src"), &cache, build_root);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/opt/pm/build/test/src"));

        // Path with .
        let result = normalize_path(Path::new("/opt/pm/build/test/./src"), &cache, build_root);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/opt/pm/build/test/src"));

        // Path with ..
        let result = normalize_path(
            Path::new("/opt/pm/build/test/src/../lib"),
            &cache,
            build_root,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/opt/pm/build/test/lib"));
    }

    #[test]
    fn test_path_traversal_detection() {
        let cache = HashMap::new();
        let build_root = Path::new("/opt/pm/build/test");

        // Relative path with too many .. components
        let result = normalize_path(Path::new("../../../../etc/passwd"), &cache, build_root);
        assert!(result.is_err());
    }

    #[test]
    fn test_safe_system_paths() {
        let build_root = Path::new("/opt/pm/build/test");

        // Safe system paths
        assert!(is_path_safe(Path::new("/usr/include/stdio.h"), build_root));
        assert!(is_path_safe(Path::new("/usr/lib/libc.so"), build_root));
        assert!(is_path_safe(Path::new("/opt/pm/live/bin/gcc"), build_root));

        // Unsafe system paths
        assert!(!is_path_safe(Path::new("/etc/passwd"), build_root));
        assert!(!is_path_safe(
            Path::new("/home/user/.ssh/id_rsa"),
            build_root
        ));
    }
}
