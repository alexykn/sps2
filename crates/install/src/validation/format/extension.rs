//! File extension validation
//!
//! This module handles validation of file extensions to ensure that
//! package files have the correct .sp extension.

use sps2_errors::{Error, InstallError};
use std::path::Path;

/// Validates that a file has the correct .sp extension
///
/// This function checks that the file path ends with the expected .sp
/// extension for package files.
///
/// # Errors
///
/// Returns an error if:
/// - File has no extension
/// - File has wrong extension (not .sp)
pub fn validate_package_extension(file_path: &Path) -> Result<(), Error> {
    if let Some(extension) = file_path.extension() {
        if extension != "sp" {
            return Err(InstallError::InvalidPackageFile {
                path: file_path.display().to_string(),
                message: format!("invalid file extension '{}'", extension.to_string_lossy()),
            }
            .into());
        }
    } else {
        return Err(InstallError::InvalidPackageFile {
            path: file_path.display().to_string(),
            message: "missing .sp file extension".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validates that a file extension is allowed
///
/// This function checks against a list of blocked extensions that
/// might indicate potentially dangerous files.
pub fn validate_allowed_extension(
    file_path: &Path,
    blocked_extensions: &[String],
) -> Result<(), Error> {
    if let Some(extension) = file_path.extension() {
        let ext_str = extension.to_string_lossy().to_lowercase();
        if blocked_extensions.contains(&ext_str) {
            return Err(InstallError::InvalidPackageFile {
                path: file_path.display().to_string(),
                message: format!("blocked file extension: {ext_str}"),
            }
            .into());
        }
    }

    Ok(())
}

/// Gets the file extension as a lowercase string
#[must_use]
pub fn get_extension(file_path: &Path) -> Option<String> {
    file_path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
}

/// Checks if a file has a specific extension
#[must_use]
pub fn has_extension(file_path: &Path, expected: &str) -> bool {
    get_extension(file_path).map_or(false, |ext| ext == expected.to_lowercase())
}

/// Common dangerous file extensions
#[must_use]
pub fn get_dangerous_extensions() -> Vec<String> {
    vec![
        "exe".to_string(),
        "bat".to_string(),
        "cmd".to_string(),
        "com".to_string(),
        "scr".to_string(),
        "pif".to_string(),
        "vbs".to_string(),
        "js".to_string(),
        "jar".to_string(),
        "app".to_string(),
        "deb".to_string(),
        "rpm".to_string(),
        "dmg".to_string(),
        "pkg".to_string(),
        "msi".to_string(),
    ]
}
