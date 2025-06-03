//! File permissions security validation
//!
//! This module provides validation of file permissions to ensure that
//! package files don't have dangerous permission settings that could
//! compromise system security.

use sps2_errors::{Error, InstallError};
use std::path::Path;

/// Validates file permissions for security
///
/// This function checks the permissions of a package file to ensure
/// it doesn't have dangerous permission bits set (setuid, setgid, etc.).
///
/// # Errors
///
/// Returns an error if:
/// - File metadata cannot be accessed
/// - File has dangerous permission bits
/// - File is not readable
pub async fn validate_file_permissions(file_path: &Path) -> Result<PermissionInfo, Error> {
    let metadata =
        tokio::fs::metadata(file_path)
            .await
            .map_err(|e| InstallError::InvalidPackageFile {
                path: file_path.display().to_string(),
                message: format!("cannot access file metadata: {e}"),
            })?;

    let mut info = PermissionInfo::new();
    let mut warnings = Vec::new();

    // On Unix, check permission bits
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();

        info.mode = Some(mode);
        info.is_readable = (mode & 0o400) != 0;
        info.is_writable = (mode & 0o200) != 0;
        info.is_executable = (mode & 0o100) != 0;
        info.has_setuid = (mode & 0o4000) != 0;
        info.has_setgid = (mode & 0o2000) != 0;
        info.has_sticky = (mode & 0o1000) != 0;

        // Check for dangerous permission bits
        if info.has_setuid {
            warnings.push("package file has setuid bit set".to_string());
        }
        if info.has_setgid {
            warnings.push("package file has setgid bit set".to_string());
        }

        // Check if file is readable
        if !info.is_readable {
            return Err(InstallError::InvalidPackageFile {
                path: file_path.display().to_string(),
                message: "file is not readable".to_string(),
            }
            .into());
        }

        // Check for world-writable files
        if (mode & 0o002) != 0 {
            warnings.push("package file is world-writable".to_string());
        }

        // Check for unusual permissions
        if (mode & 0o777) == 0o777 {
            warnings.push("package file has all permissions set (777)".to_string());
        }
    }

    // On Windows, basic checks
    #[cfg(windows)]
    {
        info.is_readable = !metadata.permissions().readonly();
        info.is_writable = !metadata.permissions().readonly();
        // Windows doesn't have executable bit in the same way
        info.is_executable = false;
    }

    info.warnings = warnings;
    Ok(info)
}

/// Validates permissions of extracted tar entry
///
/// This function checks the permissions that would be set on an extracted
/// file to ensure they are safe for the target system.
pub fn validate_tar_entry_permissions(header: &tar::Header) -> Result<PermissionInfo, Error> {
    let mut info = PermissionInfo::new();
    let mut warnings = Vec::new();

    // Get mode from tar header
    match header.mode() {
        Ok(mode) => {
            info.mode = Some(mode);
            info.is_readable = (mode & 0o400) != 0;
            info.is_writable = (mode & 0o200) != 0;
            info.is_executable = (mode & 0o100) != 0;
            info.has_setuid = (mode & 0o4000) != 0;
            info.has_setgid = (mode & 0o2000) != 0;
            info.has_sticky = (mode & 0o1000) != 0;

            // Check for dangerous permissions in extracted files
            if info.has_setuid {
                return Err(InstallError::InvalidPackageFile {
                    path: "archive".to_string(),
                    message: "setuid files not allowed in packages".to_string(),
                }
                .into());
            }

            if info.has_setgid {
                return Err(InstallError::InvalidPackageFile {
                    path: "archive".to_string(),
                    message: "setgid files not allowed in packages".to_string(),
                }
                .into());
            }

            // Warn about potentially dangerous permissions
            if (mode & 0o777) == 0o777 {
                warnings.push("file will have all permissions (777) when extracted".to_string());
            }

            if (mode & 0o002) != 0 {
                warnings.push("file will be world-writable when extracted".to_string());
            }
        }
        Err(e) => {
            warnings.push(format!("corrupted permission header: {e}"));
        }
    }

    info.warnings = warnings;
    Ok(info)
}

/// Validates that permissions are appropriate for file type
pub fn validate_permissions_for_type(entry_type: tar::EntryType, mode: u32) -> Result<(), Error> {
    match entry_type {
        tar::EntryType::Directory => {
            // Directories should be executable to be accessible
            if (mode & 0o100) == 0 {
                return Err(InstallError::InvalidPackageFile {
                    path: "archive".to_string(),
                    message: "directory is not executable (not accessible)".to_string(),
                }
                .into());
            }
        }
        tar::EntryType::Regular => {
            // Regular files don't need to be executable unless they're scripts
            // We could check file extensions here for script detection
        }
        tar::EntryType::Symlink | tar::EntryType::Link => {
            // Links inherit permissions from their targets
        }
        _ => {
            // Other types (device files, etc.) are generally not allowed
            return Err(InstallError::InvalidPackageFile {
                path: "archive".to_string(),
                message: format!("unsupported file type: {:?}", entry_type),
            }
            .into());
        }
    }

    Ok(())
}

/// Sanitizes permissions for safe extraction
///
/// This function takes a set of permissions and returns a sanitized
/// version that is safe to apply during extraction.
#[must_use]
pub fn sanitize_permissions(mode: u32) -> u32 {
    // Remove dangerous bits
    let mut sanitized = mode;

    // Remove setuid/setgid/sticky bits
    sanitized &= !0o7000;

    // Limit to reasonable permissions
    sanitized &= 0o755;

    // Ensure owner has read permission
    sanitized |= 0o400;

    sanitized
}

/// Information about file permissions
#[derive(Debug, Clone, Default)]
pub struct PermissionInfo {
    /// File mode (Unix permissions)
    pub mode: Option<u32>,
    /// Whether file is readable
    pub is_readable: bool,
    /// Whether file is writable
    pub is_writable: bool,
    /// Whether file is executable
    pub is_executable: bool,
    /// Whether file has setuid bit
    pub has_setuid: bool,
    /// Whether file has setgid bit
    pub has_setgid: bool,
    /// Whether file has sticky bit
    pub has_sticky: bool,
    /// Permission validation warnings
    pub warnings: Vec<String>,
}

impl PermissionInfo {
    /// Create new permission info
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if permissions are dangerous
    #[must_use]
    pub fn is_dangerous(&self) -> bool {
        self.has_setuid || self.has_setgid
    }

    /// Get human-readable permission string
    #[must_use]
    pub fn permission_string(&self) -> String {
        if let Some(mode) = self.mode {
            format_permissions(mode)
        } else {
            "unknown".to_string()
        }
    }

    /// Get octal representation
    #[must_use]
    pub fn octal_string(&self) -> String {
        if let Some(mode) = self.mode {
            format!("{:o}", mode & 0o7777)
        } else {
            "unknown".to_string()
        }
    }
}

/// Format permissions as rwx string
#[must_use]
pub fn format_permissions(mode: u32) -> String {
    let mut result = String::new();

    // Owner permissions
    result.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    result.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    result.push(if mode & 0o100 != 0 {
        if mode & 0o4000 != 0 {
            's'
        } else {
            'x'
        }
    } else {
        if mode & 0o4000 != 0 {
            'S'
        } else {
            '-'
        }
    });

    // Group permissions
    result.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    result.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    result.push(if mode & 0o010 != 0 {
        if mode & 0o2000 != 0 {
            's'
        } else {
            'x'
        }
    } else {
        if mode & 0o2000 != 0 {
            'S'
        } else {
            '-'
        }
    });

    // Other permissions
    result.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    result.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    result.push(if mode & 0o001 != 0 {
        if mode & 0o1000 != 0 {
            't'
        } else {
            'x'
        }
    } else {
        if mode & 0o1000 != 0 {
            'T'
        } else {
            '-'
        }
    });

    result
}

/// Permission security policy
#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    /// Allow setuid files
    pub allow_setuid: bool,
    /// Allow setgid files
    pub allow_setgid: bool,
    /// Allow world-writable files
    pub allow_world_writable: bool,
    /// Maximum allowed permissions mask
    pub max_permissions: u32,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self {
            allow_setuid: false,
            allow_setgid: false,
            allow_world_writable: false,
            max_permissions: 0o755,
        }
    }
}

impl PermissionPolicy {
    /// Create new permission policy
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set whether to allow setuid files
    #[must_use]
    pub fn with_allow_setuid(mut self, allow: bool) -> Self {
        self.allow_setuid = allow;
        self
    }

    /// Set whether to allow setgid files
    #[must_use]
    pub fn with_allow_setgid(mut self, allow: bool) -> Self {
        self.allow_setgid = allow;
        self
    }

    /// Validate permissions against this policy
    pub fn validate_permissions(&self, info: &PermissionInfo) -> Result<(), Error> {
        if !self.allow_setuid && info.has_setuid {
            return Err(InstallError::InvalidPackageFile {
                path: "package".to_string(),
                message: "setuid files not allowed by policy".to_string(),
            }
            .into());
        }

        if !self.allow_setgid && info.has_setgid {
            return Err(InstallError::InvalidPackageFile {
                path: "package".to_string(),
                message: "setgid files not allowed by policy".to_string(),
            }
            .into());
        }

        if let Some(mode) = info.mode {
            if !self.allow_world_writable && (mode & 0o002) != 0 {
                return Err(InstallError::InvalidPackageFile {
                    path: "package".to_string(),
                    message: "world-writable files not allowed by policy".to_string(),
                }
                .into());
            }

            if (mode & 0o777) > self.max_permissions {
                return Err(InstallError::InvalidPackageFile {
                    path: "package".to_string(),
                    message: format!(
                        "permissions {} exceed policy maximum {}",
                        format_permissions(mode),
                        format_permissions(self.max_permissions)
                    ),
                }
                .into());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_permissions() {
        // Remove setuid bit
        assert_eq!(sanitize_permissions(0o4755), 0o755);

        // Remove setgid bit
        assert_eq!(sanitize_permissions(0o2755), 0o755);

        // Ensure readable
        assert_eq!(sanitize_permissions(0o000), 0o400);

        // Normal permissions preserved
        assert_eq!(sanitize_permissions(0o644), 0o644);
    }

    #[test]
    fn test_format_permissions() {
        assert_eq!(format_permissions(0o755), "rwxr-xr-x");
        assert_eq!(format_permissions(0o644), "rw-r--r--");
        assert_eq!(format_permissions(0o4755), "rwsr-xr-x");
        assert_eq!(format_permissions(0o2755), "rwxr-sr-x");
        assert_eq!(format_permissions(0o1755), "rwxr-xr-t");
    }

    #[test]
    fn test_permission_info() {
        let mut info = PermissionInfo::new();
        info.mode = Some(0o755);
        info.has_setuid = true;

        assert!(info.is_dangerous());
        assert_eq!(info.permission_string(), "rwxr-xr-x");
        assert_eq!(info.octal_string(), "755");
    }

    #[test]
    fn test_permission_policy() {
        let policy = PermissionPolicy::new();

        let mut safe_info = PermissionInfo::new();
        safe_info.mode = Some(0o644);
        assert!(policy.validate_permissions(&safe_info).is_ok());

        let mut dangerous_info = PermissionInfo::new();
        dangerous_info.has_setuid = true;
        assert!(policy.validate_permissions(&dangerous_info).is_err());
    }

    #[test]
    fn test_validate_permissions_for_type() {
        // Directory should be executable
        assert!(validate_permissions_for_type(tar::EntryType::Directory, 0o755).is_ok());
        assert!(validate_permissions_for_type(tar::EntryType::Directory, 0o644).is_err());

        // Regular file doesn't need to be executable
        assert!(validate_permissions_for_type(tar::EntryType::Regular, 0o644).is_ok());
        assert!(validate_permissions_for_type(tar::EntryType::Regular, 0o755).is_ok());
    }
}
