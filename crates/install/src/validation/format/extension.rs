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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_validate_package_extension_valid() {
        let path = PathBuf::from("test.sp");
        assert!(validate_package_extension(&path).is_ok());
    }

    #[test]
    fn test_validate_package_extension_invalid() {
        let path = PathBuf::from("test.txt");
        let result = validate_package_extension(&path);
        assert!(result.is_err());

        if let Err(e) = result {
            let error_message = e.to_string();
            assert!(error_message.contains("invalid file extension"));
        }
    }

    #[test]
    fn test_validate_package_extension_missing() {
        let path = PathBuf::from("test");
        let result = validate_package_extension(&path);
        assert!(result.is_err());

        if let Err(e) = result {
            let error_message = e.to_string();
            assert!(error_message.contains("missing .sp file extension"));
        }
    }

    #[test]
    fn test_validate_allowed_extension() {
        let path = PathBuf::from("script.sh");
        let blocked = vec!["exe".to_string(), "bat".to_string()];

        // Should be allowed
        assert!(validate_allowed_extension(&path, &blocked).is_ok());

        // Should be blocked
        let blocked_path = PathBuf::from("malware.exe");
        assert!(validate_allowed_extension(&blocked_path, &blocked).is_err());
    }

    #[test]
    fn test_get_extension() {
        assert_eq!(
            get_extension(&PathBuf::from("test.SP")),
            Some("sp".to_string())
        );
        assert_eq!(
            get_extension(&PathBuf::from("test.TXT")),
            Some("txt".to_string())
        );
        assert_eq!(get_extension(&PathBuf::from("test")), None);
    }

    #[test]
    fn test_has_extension() {
        assert!(has_extension(&PathBuf::from("test.sp"), "sp"));
        assert!(has_extension(&PathBuf::from("test.SP"), "sp"));
        assert!(!has_extension(&PathBuf::from("test.txt"), "sp"));
        assert!(!has_extension(&PathBuf::from("test"), "sp"));
    }

    #[test]
    fn test_dangerous_extensions() {
        let dangerous = get_dangerous_extensions();
        assert!(dangerous.contains(&"exe".to_string()));
        assert!(dangerous.contains(&"bat".to_string()));
        assert!(!dangerous.contains(&"txt".to_string()));
    }
}
