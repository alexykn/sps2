//! Path security validation
//!
//! This module provides security validation for file paths within packages
//! to prevent path traversal attacks, directory climbing, and other
//! path-based security vulnerabilities.

use sps2_errors::{Error, InstallError};
use std::path::{Component, Path, PathBuf};

/// Validates that a path is safe for extraction
///
/// This function checks for various path-based security vulnerabilities:
/// - Path traversal attempts (../ sequences)
/// - Absolute paths
/// - Suspicious path components
/// - Overly long paths
///
/// # Errors
///
/// Returns an error if the path is potentially dangerous.
pub fn validate_safe_path(path: &str) -> Result<(), Error> {
    if path.is_empty() {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: "empty path not allowed".to_string(),
        }
        .into());
    }

    // Check for path traversal attempts
    if path.contains("..") {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!("path traversal attempt detected: {path}"),
        }
        .into());
    }

    // Convert to Path for more detailed checking
    let path_buf = PathBuf::from(path);

    // Check if path is absolute
    if path_buf.is_absolute() {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!("absolute path not allowed: {path}"),
        }
        .into());
    }

    // Check individual path components
    for component in path_buf.components() {
        match component {
            Component::Normal(name) => {
                validate_path_component(&name.to_string_lossy())?;
            }
            Component::ParentDir => {
                return Err(InstallError::InvalidPackageFile {
                    path: "package".to_string(),
                    message: format!("parent directory reference not allowed: {path}"),
                }
                .into());
            }
            Component::RootDir => {
                return Err(InstallError::InvalidPackageFile {
                    path: "package".to_string(),
                    message: format!("root directory reference not allowed: {path}"),
                }
                .into());
            }
            Component::CurDir => {
                // Current directory references are generally safe but discouraged
                continue;
            }
            Component::Prefix(_) => {
                return Err(InstallError::InvalidPackageFile {
                    path: "package".to_string(),
                    message: format!("path prefix not allowed: {path}"),
                }
                .into());
            }
        }
    }

    Ok(())
}

/// Validates an individual path component
///
/// Checks for suspicious characters or patterns in individual
/// path components that might indicate security issues.
fn validate_path_component(component: &str) -> Result<(), Error> {
    if component.is_empty() {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: "empty path component not allowed".to_string(),
        }
        .into());
    }

    // Check for suspicious characters
    let suspicious_chars = ['\0', '\x01', '\x02', '\x03', '\x04', '\x05', '\x06', '\x07'];
    for &bad_char in &suspicious_chars {
        if component.contains(bad_char) {
            return Err(InstallError::InvalidPackageFile {
                path: "package".to_string(),
                message: format!("suspicious character in path component: {component}"),
            }
            .into());
        }
    }

    // Check for Windows device names (even on Unix for cross-platform safety)
    let windows_devices = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    let component_upper = component.to_uppercase();
    for device in &windows_devices {
        if component_upper == *device || component_upper.starts_with(&format!("{device}.")) {
            return Err(InstallError::InvalidPackageFile {
                path: "package".to_string(),
                message: format!("Windows device name not allowed: {component}"),
            }
            .into());
        }
    }

    // Check component length
    if component.len() > 255 {
        return Err(InstallError::InvalidPackageFile {
            path: "package".to_string(),
            message: format!("path component too long: {} characters", component.len()),
        }
        .into());
    }

    Ok(())
}

/// Normalizes a path by removing redundant components
///
/// This function creates a normalized version of the path that can be
/// safely used for extraction while preserving the intended structure.
#[must_use]
pub fn normalize_path(path: &str) -> PathBuf {
    let path_buf = PathBuf::from(path);
    let mut normalized = PathBuf::new();

    for component in path_buf.components() {
        match component {
            Component::Normal(name) => {
                normalized.push(name);
            }
            Component::CurDir => {
                // Skip current directory references
                continue;
            }
            // All other components should have been caught by validate_safe_path
            _ => {
                // This shouldn't happen if validation was done first
                continue;
            }
        }
    }

    normalized
}

/// Checks if a path would escape the extraction directory
///
/// This function checks if resolving the given path relative to a base
/// directory would result in a path outside the base directory.
pub fn would_escape_directory(base: &Path, relative_path: &str) -> Result<bool, Error> {
    let path_buf = PathBuf::from(relative_path);

    // First validate the path is safe
    validate_safe_path(relative_path)?;

    // Resolve relative to base
    let resolved = base.join(&path_buf);

    // Check if the resolved path starts with the base path
    match resolved.canonicalize() {
        Ok(canonical) => {
            match base.canonicalize() {
                Ok(canonical_base) => Ok(!canonical.starts_with(canonical_base)),
                Err(_) => {
                    // If base doesn't exist, do a simpler check
                    Ok(!resolved.starts_with(base))
                }
            }
        }
        Err(_) => {
            // If path doesn't exist yet, do a simpler check
            Ok(!resolved.starts_with(base))
        }
    }
}

/// Common dangerous path patterns
#[must_use]
pub fn get_dangerous_path_patterns() -> Vec<String> {
    vec![
        "..".to_string(),
        "./.".to_string(),
        "../".to_string(),
        "/..".to_string(),
        "\\..".to_string(),
        "%2e%2e".to_string(),     // URL encoded ..
        "%252e%252e".to_string(), // Double URL encoded ..
        "/etc/".to_string(),
        "/usr/".to_string(),
        "/var/".to_string(),
        "/root/".to_string(),
        "/home/".to_string(),
        "C:".to_string(),
        "\\Windows\\".to_string(),
        "\\System32\\".to_string(),
    ]
}

/// Path security policy
#[derive(Debug, Clone)]
pub struct PathSecurityPolicy {
    /// Allow current directory references
    pub allow_current_dir: bool,
    /// Maximum path component length
    pub max_component_length: usize,
    /// Additional blocked patterns
    pub blocked_patterns: Vec<String>,
    /// Case sensitive pattern matching
    pub case_sensitive: bool,
}

impl Default for PathSecurityPolicy {
    fn default() -> Self {
        Self {
            allow_current_dir: true,
            max_component_length: 255,
            blocked_patterns: get_dangerous_path_patterns(),
            case_sensitive: false,
        }
    }
}

impl PathSecurityPolicy {
    /// Create new path security policy
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set whether to allow current directory references
    #[must_use]
    pub fn with_allow_current_dir(mut self, allow: bool) -> Self {
        self.allow_current_dir = allow;
        self
    }

    /// Add blocked pattern
    #[must_use]
    pub fn with_blocked_pattern(mut self, pattern: String) -> Self {
        self.blocked_patterns.push(pattern);
        self
    }

    /// Validate path against this policy
    pub fn validate_path(&self, path: &str) -> Result<(), Error> {
        // First do basic safety validation
        validate_safe_path(path)?;

        // Check against blocked patterns
        for pattern in &self.blocked_patterns {
            let matches = if self.case_sensitive {
                path.contains(pattern)
            } else {
                path.to_lowercase().contains(&pattern.to_lowercase())
            };

            if matches {
                return Err(InstallError::InvalidPackageFile {
                    path: "package".to_string(),
                    message: format!("path contains blocked pattern '{pattern}': {path}"),
                }
                .into());
            }
        }

        Ok(())
    }
}
