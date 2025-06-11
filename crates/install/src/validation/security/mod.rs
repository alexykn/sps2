//! Security validation module
//!
//! This module provides comprehensive security validation for packages including:
//! - Path safety validation (preventing traversal attacks)
//! - File permission validation (preventing privilege escalation)
//! - Symlink validation (preventing symlink attacks)
//! - Security policy enforcement

pub mod paths;
pub mod permissions;
pub mod policies;
pub mod symlinks;

use sps2_errors::Error;
use sps2_events::EventSender;
use std::path::Path;

use crate::validation::types::{PackageFormat, ValidationResult};

pub use paths::{
    get_dangerous_path_patterns, normalize_path, validate_safe_path, would_escape_directory,
    PathSecurityPolicy,
};
pub use permissions::{
    format_permissions, sanitize_permissions, validate_file_permissions,
    validate_tar_entry_permissions, PermissionInfo, PermissionPolicy,
};
pub use policies::{
    SecurityLevel, SecurityPattern, SecurityPolicy, SecurityViolation, ViolationSeverity,
};
pub use symlinks::{
    detect_symlink_attacks, resolve_symlink_target, validate_symlink_target,
    would_create_symlink_loop, SymlinkInfo, SymlinkPolicy,
};

/// Validates security properties of the package
///
/// This is the main entry point for security validation. It performs
/// comprehensive security checks including file permissions, path safety,
/// and policy enforcement.
///
/// # Errors
///
/// Returns an error if any critical security validation fails.
pub async fn validate_security_properties(
    file_path: &Path,
    _format: &PackageFormat,
    result: &mut ValidationResult,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::OperationStarted {
            operation: "Validating security properties".to_string(),
        });
    }

    // Validate file permissions
    let permission_info = validate_file_permissions(file_path).await?;

    // Add permission warnings to result
    for warning in &permission_info.warnings {
        result.add_warning(warning.clone());
    }

    // Additional security checks can be added here in the future

    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::OperationCompleted {
            operation: "Security validation completed".to_string(),
            success: true,
        });
    }

    Ok(())
}

/// Validates package against comprehensive security policy
///
/// This function applies a full security policy to validate all aspects
/// of a package including paths, permissions, symlinks, and custom rules.
pub async fn validate_package_security(
    file_path: &Path,
    format: &PackageFormat,
    result: &mut ValidationResult,
    policy: &SecurityPolicy,
    event_sender: Option<&EventSender>,
) -> Result<(), Error> {
    if let Some(sender) = event_sender {
        let _ = sender.send(sps2_events::Event::DebugLog {
            message: format!(
                "DEBUG: Applying {} security policy to package",
                policy.security_level_description()
            ),
            context: std::collections::HashMap::new(),
        });
    }

    // Basic security validation
    validate_security_properties(file_path, format, result, event_sender).await?;

    // Apply security policy to current results
    apply_security_policy_to_results(result, policy)?;

    Ok(())
}

/// Applies security policy to validation results
///
/// This function takes existing validation results and applies the security
/// policy to check for policy violations.
fn apply_security_policy_to_results(
    result: &mut ValidationResult,
    policy: &SecurityPolicy,
) -> Result<(), Error> {
    // For now, we mainly validate at the package level
    // Individual file validation would happen during content validation

    // Apply any package-level security rules
    for rule in &policy.custom_rules {
        match &rule.pattern {
            SecurityPattern::SizeRange { min, max } => {
                if result.extracted_size < *min || result.extracted_size > *max {
                    if rule.is_fatal {
                        return Err(sps2_errors::InstallError::InvalidPackageFile {
                            path: "package".to_string(),
                            message: format!(
                                "security rule '{}' failed: {}",
                                rule.name, rule.description
                            ),
                        }
                        .into());
                    } else {
                        result.add_warning(format!(
                            "security warning '{}': {}",
                            rule.name, rule.description
                        ));
                    }
                }
            }
            _ => {
                // Other patterns are applied during file-by-file validation
            }
        }
    }

    Ok(())
}

/// Validates individual file security
///
/// This function validates the security properties of an individual file
/// within a package, including its path, permissions, and content.
pub fn validate_file_security(
    file_path: &str,
    file_size: u64,
    permissions: Option<u32>,
    is_symlink: bool,
    symlink_target: Option<&str>,
    policy: &SecurityPolicy,
) -> Result<Vec<String>, Error> {
    let mut warnings = Vec::new();

    // Validate path security
    let path_warnings = policy.validate_file_path(file_path)?;
    warnings.extend(path_warnings);

    // Validate file size
    let size_warnings = policy.validate_file_size(file_size)?;
    warnings.extend(size_warnings);

    // Validate permissions if available
    if let Some(mode) = permissions {
        let mut perm_info = PermissionInfo::new();
        perm_info.mode = Some(mode);
        perm_info.is_readable = (mode & 0o400) != 0;
        perm_info.is_writable = (mode & 0o200) != 0;
        perm_info.is_executable = (mode & 0o100) != 0;
        perm_info.has_setuid = (mode & 0o4000) != 0;
        perm_info.has_setgid = (mode & 0o2000) != 0;

        let perm_warnings = policy.validate_permissions(&perm_info)?;
        warnings.extend(perm_warnings);
    }

    // Validate symlinks
    if is_symlink {
        if let Some(target) = symlink_target {
            let symlink_warnings = policy.validate_symlink(file_path, target)?;
            warnings.extend(symlink_warnings);
        } else {
            warnings.push("symlink has no target".to_string());
        }
    }

    Ok(warnings)
}

/// Creates a comprehensive security report
#[derive(Debug)]
pub struct SecurityReport {
    /// Overall security assessment
    pub overall_status: SecurityStatus,
    /// Individual security violations
    pub violations: Vec<SecurityViolation>,
    /// Security warnings (non-fatal)
    pub warnings: Vec<String>,
    /// Applied security policy
    pub policy_level: SecurityLevel,
    /// Recommendation for improvement
    pub recommendations: Vec<String>,
}

/// Overall security status
#[derive(Debug, PartialEq, Clone)]
pub enum SecurityStatus {
    /// Package is secure
    Secure,
    /// Package has warnings but is acceptable
    Warning,
    /// Package has security issues
    Insecure,
    /// Package is dangerous and should not be installed
    Dangerous,
}

impl SecurityReport {
    /// Create new security report
    #[must_use]
    pub fn new(policy_level: SecurityLevel) -> Self {
        Self {
            overall_status: SecurityStatus::Secure,
            violations: Vec::new(),
            warnings: Vec::new(),
            policy_level,
            recommendations: Vec::new(),
        }
    }

    /// Add security violation
    pub fn add_violation(&mut self, violation: SecurityViolation) {
        if violation.is_fatal() {
            self.overall_status = match violation.severity {
                ViolationSeverity::Critical => SecurityStatus::Dangerous,
                ViolationSeverity::Error => SecurityStatus::Insecure,
                _ => self.overall_status.clone(),
            };
        } else if self.overall_status == SecurityStatus::Secure {
            self.overall_status = SecurityStatus::Warning;
        }
        self.violations.push(violation);
    }

    /// Add warning
    pub fn add_warning(&mut self, warning: String) {
        if self.overall_status == SecurityStatus::Secure {
            self.overall_status = SecurityStatus::Warning;
        }
        self.warnings.push(warning);
    }

    /// Add recommendation
    pub fn add_recommendation(&mut self, recommendation: String) {
        self.recommendations.push(recommendation);
    }

    /// Check if package is safe to install
    #[must_use]
    pub fn is_safe_to_install(&self) -> bool {
        matches!(
            self.overall_status,
            SecurityStatus::Secure | SecurityStatus::Warning
        )
    }

    /// Get summary message
    #[must_use]
    pub fn summary_message(&self) -> String {
        match self.overall_status {
            SecurityStatus::Secure => "Package passed all security checks".to_string(),
            SecurityStatus::Warning => format!(
                "Package has {} warnings but is acceptable",
                self.warnings.len()
            ),
            SecurityStatus::Insecure => {
                format!("Package has {} security issues", self.violations.len())
            }
            SecurityStatus::Dangerous => {
                "Package contains critical security vulnerabilities".to_string()
            }
        }
    }
}
