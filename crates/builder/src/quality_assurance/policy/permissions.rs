//! File permission validation

use super::{should_exclude_file, PolicyValidator};
use crate::quality_assurance::types::{PolicyRule, QaCheck, QaCheckType, QaSeverity};
use crate::BuildContext;
use sps2_errors::Error;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tokio::fs;

/// File permission validator
pub struct PermissionValidator;

impl PermissionValidator {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for PermissionValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl PolicyValidator for PermissionValidator {
    fn id(&self) -> &'static str {
        "file-permissions"
    }

    fn name(&self) -> &'static str {
        "File Permission Check"
    }

    async fn validate(
        &self,
        _context: &BuildContext,
        path: &Path,
        rule: &PolicyRule,
    ) -> Result<Vec<QaCheck>, Error> {
        let mut checks = Vec::new();

        // Get configuration
        let check_world_writable = rule
            .config
            .get("check_world_writable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        let check_setuid_setgid = rule
            .config
            .get("check_setuid_setgid")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        let max_permissions = rule
            .config
            .get("max_permissions")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(0o755); // Default max permissions

        let exclude_patterns = rule
            .config
            .get("exclude_patterns")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Scan directory recursively
        checks.extend(
            scan_permissions_recursive(
                path,
                check_world_writable,
                check_setuid_setgid,
                max_permissions,
                &exclude_patterns,
                rule.severity,
            )
            .await?,
        );

        Ok(checks)
    }
}

/// Recursively scan directory for permission issues
fn scan_permissions_recursive<'a>(
    dir: &'a Path,
    check_world_writable: bool,
    check_setuid_setgid: bool,
    max_permissions: u32,
    exclude_patterns: &'a [String],
    severity: QaSeverity,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<QaCheck>, Error>> + Send + 'a>> {
    Box::pin(async move {
        let mut checks = Vec::new();

        // Skip if this path should be excluded
        if should_exclude_file(dir, exclude_patterns) {
            return Ok(checks);
        }

        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Skip excluded paths
            if should_exclude_file(&path, exclude_patterns) {
                continue;
            }

            let metadata = entry.metadata().await?;
            let permissions = metadata.permissions();
            let mode = permissions.mode();

            // Perform all permission checks for this entry
            checks.extend(check_entry_permissions(
                &path,
                mode,
                check_world_writable,
                check_setuid_setgid,
                max_permissions,
                severity,
            ));

            // Recurse into subdirectories
            if path.is_dir() {
                let mut sub_checks = scan_permissions_recursive(
                    &path,
                    check_world_writable,
                    check_setuid_setgid,
                    max_permissions,
                    exclude_patterns,
                    severity,
                )
                .await?;
                checks.append(&mut sub_checks);
            }
        }

        Ok(checks)
    })
}

/// Check permissions for a single file/directory entry
fn check_entry_permissions(
    path: &Path,
    mode: u32,
    check_world_writable: bool,
    check_setuid_setgid: bool,
    max_permissions: u32,
    severity: QaSeverity,
) -> Vec<QaCheck> {
    let mut checks = Vec::new();

    // Check for world-writable files
    if check_world_writable && (mode & 0o002) != 0 {
        checks.push(create_world_writable_check(path, mode, severity));
    }

    // Check for setuid/setgid bits
    if check_setuid_setgid {
        checks.extend(check_setuid_setgid_bits(path, mode, severity));
    }

    // Check if permissions exceed maximum allowed
    if let Some(check) = check_max_permissions(path, mode, max_permissions) {
        checks.push(check);
    }

    // Check special file cases
    if path.is_file() {
        checks.extend(check_special_file_permissions(path, mode, severity));
    }

    checks
}

/// Create check for world-writable file
fn create_world_writable_check(path: &Path, mode: u32, severity: QaSeverity) -> QaCheck {
    QaCheck::new(
        QaCheckType::PermissionCheck,
        "file-permissions",
        severity,
        format!("World-writable file detected (mode: {:o})", mode & 0o777),
    )
    .with_location(path.to_path_buf(), None, None)
    .with_context("World-writable files are a security risk")
}

/// Check for setuid/setgid bits
fn check_setuid_setgid_bits(path: &Path, mode: u32, severity: QaSeverity) -> Vec<QaCheck> {
    let mut checks = Vec::new();

    if (mode & 0o4000) != 0 {
        checks.push(
            QaCheck::new(
                QaCheckType::PermissionCheck,
                "file-permissions",
                severity,
                "File has setuid bit set",
            )
            .with_location(path.to_path_buf(), None, None)
            .with_context("Setuid files can be a security risk"),
        );
    }

    if (mode & 0o2000) != 0 {
        checks.push(
            QaCheck::new(
                QaCheckType::PermissionCheck,
                "file-permissions",
                severity,
                "File has setgid bit set",
            )
            .with_location(path.to_path_buf(), None, None)
            .with_context("Setgid files can be a security risk"),
        );
    }

    checks
}

/// Check if permissions exceed maximum allowed
fn check_max_permissions(path: &Path, mode: u32, max_permissions: u32) -> Option<QaCheck> {
    let file_perms = mode & 0o777;
    if file_perms > max_permissions {
        Some(
            QaCheck::new(
                QaCheckType::PermissionCheck,
                "file-permissions",
                QaSeverity::Warning,
                format!(
                    "File permissions ({:o}) exceed maximum allowed ({:o})",
                    file_perms, max_permissions
                ),
            )
            .with_location(path.to_path_buf(), None, None),
        )
    } else {
        None
    }
}

/// Check special file permission cases
fn check_special_file_permissions(path: &Path, mode: u32, severity: QaSeverity) -> Vec<QaCheck> {
    let mut checks = Vec::new();
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Check executable scripts have appropriate permissions
    if is_script_file(filename) && (mode & 0o111) == 0 {
        checks.push(
            QaCheck::new(
                QaCheckType::PermissionCheck,
                "file-permissions",
                QaSeverity::Info,
                "Script file is not executable",
            )
            .with_location(path.to_path_buf(), None, None)
            .with_context("Script files should typically be executable"),
        );
    }

    // Check sensitive files have restrictive permissions
    if is_sensitive_file(path) && (mode & 0o077) != 0 {
        checks.push(
            QaCheck::new(
                QaCheckType::PermissionCheck,
                "file-permissions",
                severity,
                format!("Sensitive file has loose permissions ({:o})", mode & 0o777),
            )
            .with_location(path.to_path_buf(), None, None)
            .with_context("Sensitive files should not be readable by group/others"),
        );
    }

    checks
}

/// Check if file is a script
fn is_script_file(filename: &str) -> bool {
    filename.ends_with(".sh") || filename.ends_with(".py") || filename.ends_with(".rb")
}

/// Check if a file is considered sensitive
fn is_sensitive_file(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        // Common sensitive file patterns
        name.contains("private")
            || name.contains("secret")
            || name.contains("key")
            || name.contains("password")
            || name.contains("token")
            || name.contains("credentials")
            || name == ".env"
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".pfx")
            || name.ends_with(".p12")
            || name == "id_rsa"
            || name == "id_dsa"
            || name == "id_ecdsa"
            || name == "id_ed25519"
    } else {
        false
    }
}
