//! Security policies for package validation
//!
//! This module defines comprehensive security policies that can be applied
//! during package validation to enforce organizational security standards
//! and prevent various types of attacks.

use sps2_errors::{Error, InstallError};

use super::paths::PathSecurityPolicy;
use super::permissions::PermissionPolicy;
use super::symlinks::SymlinkPolicy;

/// Comprehensive security policy for package validation
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Path security policy
    pub path_policy: PathSecurityPolicy,
    /// Permission security policy
    pub permission_policy: PermissionPolicy,
    /// Symlink security policy
    pub symlink_policy: SymlinkPolicy,
    /// Overall security level
    pub security_level: SecurityLevel,
    /// Additional custom rules
    pub custom_rules: Vec<CustomSecurityRule>,
}

/// Security levels for package validation
#[derive(Debug, Clone, PartialEq)]
pub enum SecurityLevel {
    /// Minimal security checks
    Permissive,
    /// Standard security checks (default)
    Standard,
    /// Strict security checks
    Strict,
    /// Maximum security (paranoid mode)
    Paranoid,
}

/// Custom security rule
#[derive(Debug, Clone)]
pub struct CustomSecurityRule {
    /// Rule name
    pub name: String,
    /// Rule description
    pub description: String,
    /// Whether rule failure is fatal (error) or warning
    pub is_fatal: bool,
    /// Rule pattern to match against
    pub pattern: SecurityPattern,
}

/// Security pattern for custom rules
#[derive(Debug, Clone)]
pub enum SecurityPattern {
    /// Match file path pattern
    PathPattern(String),
    /// Match file content pattern
    ContentPattern(String),
    /// Match file size range
    SizeRange { min: u64, max: u64 },
    /// Match permission bits
    PermissionBits(u32),
    /// Custom validation function name
    Custom(String),
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

impl SecurityPolicy {
    /// Create a new security policy with standard settings
    #[must_use]
    pub fn standard() -> Self {
        Self {
            path_policy: PathSecurityPolicy::default(),
            permission_policy: PermissionPolicy::default(),
            symlink_policy: SymlinkPolicy::default(),
            security_level: SecurityLevel::Standard,
            custom_rules: Vec::new(),
        }
    }

    /// Create a permissive security policy
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            path_policy: PathSecurityPolicy::new().with_allow_current_dir(true),
            permission_policy: PermissionPolicy::new(),
            symlink_policy: SymlinkPolicy::new().with_allow_symlinks(true),
            security_level: SecurityLevel::Permissive,
            custom_rules: Vec::new(),
        }
    }

    /// Create a strict security policy
    #[must_use]
    pub fn strict() -> Self {
        Self {
            path_policy: PathSecurityPolicy::new()
                .with_allow_current_dir(false)
                .with_blocked_pattern("tmp".to_string())
                .with_blocked_pattern("temp".to_string()),
            permission_policy: PermissionPolicy::new()
                .with_allow_setuid(false)
                .with_allow_setgid(false),
            symlink_policy: SymlinkPolicy::new()
                .with_allow_symlinks(true)
                .with_max_chain_length(5),
            security_level: SecurityLevel::Strict,
            custom_rules: vec![
                CustomSecurityRule {
                    name: "no_executable_scripts".to_string(),
                    description: "Prevent executable script files".to_string(),
                    is_fatal: true,
                    pattern: SecurityPattern::PathPattern("*.sh".to_string()),
                },
                CustomSecurityRule {
                    name: "size_limit".to_string(),
                    description: "Individual file size limit".to_string(),
                    is_fatal: true,
                    pattern: SecurityPattern::SizeRange {
                        min: 0,
                        max: 50 * 1024 * 1024, // 50MB
                    },
                },
            ],
        }
    }

    /// Create a paranoid security policy
    #[must_use]
    pub fn paranoid() -> Self {
        Self {
            path_policy: PathSecurityPolicy::new()
                .with_allow_current_dir(false)
                .with_blocked_pattern("tmp".to_string())
                .with_blocked_pattern("temp".to_string())
                .with_blocked_pattern("cache".to_string())
                .with_blocked_pattern("log".to_string()),
            permission_policy: PermissionPolicy::new()
                .with_allow_setuid(false)
                .with_allow_setgid(false),
            symlink_policy: SymlinkPolicy::new().with_allow_symlinks(false),
            security_level: SecurityLevel::Paranoid,
            custom_rules: vec![
                CustomSecurityRule {
                    name: "no_executables".to_string(),
                    description: "No executable files allowed".to_string(),
                    is_fatal: true,
                    pattern: SecurityPattern::PermissionBits(0o100),
                },
                CustomSecurityRule {
                    name: "strict_size_limit".to_string(),
                    description: "Very strict file size limit".to_string(),
                    is_fatal: true,
                    pattern: SecurityPattern::SizeRange {
                        min: 0,
                        max: 10 * 1024 * 1024, // 10MB
                    },
                },
            ],
        }
    }

    /// Set security level
    #[must_use]
    pub fn with_security_level(mut self, level: SecurityLevel) -> Self {
        self.security_level = level;
        self
    }

    /// Add custom security rule
    #[must_use]
    pub fn with_custom_rule(mut self, rule: CustomSecurityRule) -> Self {
        self.custom_rules.push(rule);
        self
    }

    /// Set path policy
    #[must_use]
    pub fn with_path_policy(mut self, policy: PathSecurityPolicy) -> Self {
        self.path_policy = policy;
        self
    }

    /// Set permission policy
    #[must_use]
    pub fn with_permission_policy(mut self, policy: PermissionPolicy) -> Self {
        self.permission_policy = policy;
        self
    }

    /// Set symlink policy
    #[must_use]
    pub fn with_symlink_policy(mut self, policy: SymlinkPolicy) -> Self {
        self.symlink_policy = policy;
        self
    }

    /// Validate a file path against all policies
    pub fn validate_file_path(&self, path: &str) -> Result<Vec<String>, Error> {
        let mut warnings = Vec::new();

        // Path policy validation
        self.path_policy.validate_path(path)?;

        // Apply custom rules
        for rule in &self.custom_rules {
            match &rule.pattern {
                SecurityPattern::PathPattern(pattern) => {
                    if path_matches_pattern(path, pattern) {
                        if rule.is_fatal {
                            return Err(InstallError::InvalidPackageFile {
                                path: "package".to_string(),
                                message: format!(
                                    "security rule '{}' failed: {}",
                                    rule.name, rule.description
                                ),
                            }
                            .into());
                        } else {
                            warnings.push(format!(
                                "security warning '{}': {}",
                                rule.name, rule.description
                            ));
                        }
                    }
                }
                _ => {
                    // Other patterns handled elsewhere
                }
            }
        }

        Ok(warnings)
    }

    /// Validate file permissions against policies
    pub fn validate_permissions(
        &self,
        info: &super::permissions::PermissionInfo,
    ) -> Result<Vec<String>, Error> {
        let mut warnings = Vec::new();

        // Permission policy validation
        self.permission_policy.validate_permissions(info)?;

        // Apply custom permission rules
        for rule in &self.custom_rules {
            if let SecurityPattern::PermissionBits(required_bits) = rule.pattern {
                if let Some(mode) = info.mode {
                    if (mode & required_bits) != 0 {
                        if rule.is_fatal {
                            return Err(InstallError::InvalidPackageFile {
                                path: "package".to_string(),
                                message: format!(
                                    "security rule '{}' failed: {}",
                                    rule.name, rule.description
                                ),
                            }
                            .into());
                        } else {
                            warnings.push(format!(
                                "security warning '{}': {}",
                                rule.name, rule.description
                            ));
                        }
                    }
                }
            }
        }

        Ok(warnings)
    }

    /// Validate symlink against policies
    pub fn validate_symlink(&self, link_path: &str, target: &str) -> Result<Vec<String>, Error> {
        let warnings = Vec::new();

        // Symlink policy validation
        self.symlink_policy.validate_symlink(link_path, target)?;

        // Custom symlink rules would go here

        Ok(warnings)
    }

    /// Validate file size against policies
    pub fn validate_file_size(&self, size: u64) -> Result<Vec<String>, Error> {
        let mut warnings = Vec::new();

        // Apply custom size rules
        for rule in &self.custom_rules {
            if let SecurityPattern::SizeRange { min, max } = rule.pattern {
                if size < min || size > max {
                    if rule.is_fatal {
                        return Err(InstallError::InvalidPackageFile {
                            path: "package".to_string(),
                            message: format!(
                                "security rule '{}' failed: {}",
                                rule.name, rule.description
                            ),
                        }
                        .into());
                    } else {
                        warnings.push(format!(
                            "security warning '{}': {}",
                            rule.name, rule.description
                        ));
                    }
                }
            }
        }

        Ok(warnings)
    }

    /// Get security level description
    #[must_use]
    pub fn security_level_description(&self) -> &'static str {
        match self.security_level {
            SecurityLevel::Permissive => "Minimal security checks for development",
            SecurityLevel::Standard => "Standard security checks for normal use",
            SecurityLevel::Strict => "Strict security checks for production",
            SecurityLevel::Paranoid => "Maximum security checks for high-security environments",
        }
    }

    /// Check if policy allows potentially dangerous operations
    #[must_use]
    pub fn is_permissive(&self) -> bool {
        matches!(self.security_level, SecurityLevel::Permissive)
    }

    /// Check if policy enforces strict security
    #[must_use]
    pub fn is_strict(&self) -> bool {
        matches!(
            self.security_level,
            SecurityLevel::Strict | SecurityLevel::Paranoid
        )
    }
}

/// Simple pattern matching for paths
fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    if pattern.contains('*') {
        // Simple glob matching
        if pattern.starts_with("*.") {
            let extension = &pattern[2..];
            path.ends_with(extension)
        } else if pattern.ends_with("*") {
            let prefix = &pattern[..pattern.len() - 1];
            path.starts_with(prefix)
        } else {
            // More complex patterns would need a proper glob library
            path.contains(&pattern.replace('*', ""))
        }
    } else {
        path.contains(pattern)
    }
}

/// Security violation information
#[derive(Debug, Clone)]
pub struct SecurityViolation {
    /// Rule that was violated
    pub rule_name: String,
    /// Violation description
    pub description: String,
    /// Severity level
    pub severity: ViolationSeverity,
    /// Context information
    pub context: std::collections::HashMap<String, String>,
}

/// Severity of security violations
#[derive(Debug, Clone, PartialEq)]
pub enum ViolationSeverity {
    /// Informational message
    Info,
    /// Warning about potential issue
    Warning,
    /// Error that prevents installation
    Error,
    /// Critical security issue
    Critical,
}

impl SecurityViolation {
    /// Create new security violation
    #[must_use]
    pub fn new(rule_name: String, description: String, severity: ViolationSeverity) -> Self {
        Self {
            rule_name,
            description,
            severity,
            context: std::collections::HashMap::new(),
        }
    }

    /// Add context information
    #[must_use]
    pub fn with_context(mut self, key: String, value: String) -> Self {
        self.context.insert(key, value);
        self
    }

    /// Check if violation is fatal
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        matches!(
            self.severity,
            ViolationSeverity::Error | ViolationSeverity::Critical
        )
    }
}
