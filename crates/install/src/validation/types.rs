//! Shared types and constants for package validation
//!
//! This module defines the common types, constants, and validation result
//! structures used throughout the validation system.

/// Maximum allowed size for a .sp file (500MB)
pub const MAX_PACKAGE_SIZE: u64 = 500 * 1024 * 1024;

/// Maximum allowed size for extracted content (1GB)
pub const MAX_EXTRACTED_SIZE: u64 = 1024 * 1024 * 1024;

/// Maximum number of files in a package
pub const MAX_FILE_COUNT: usize = 100_000;

/// Maximum path length to prevent path-based attacks
pub const MAX_PATH_LENGTH: usize = 4096;

/// Zstd magic bytes: 0xFD2FB528 (little-endian: 0x28, 0xB5, 0x2F, 0xFD)
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Package file format
#[derive(Debug, Clone, PartialEq)]
pub enum PackageFormat {
    /// Zstd-compressed tar archive
    ZstdCompressed,
    /// Plain tar archive
    PlainTar,
    /// Unknown/invalid format
    Unknown,
}

/// Result of package validation
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the package is valid
    pub is_valid: bool,
    /// Package format (zstd-compressed or plain tar)
    pub format: PackageFormat,
    /// Detected file count
    pub file_count: usize,
    /// Estimated extracted size
    pub extracted_size: u64,
    /// Validation warnings (non-fatal issues)
    pub warnings: Vec<String>,
    /// Manifest content if successfully parsed
    pub manifest: Option<String>,
}

impl ValidationResult {
    /// Create a new validation result
    #[must_use]
    pub fn new(format: PackageFormat) -> Self {
        Self {
            is_valid: false,
            format,
            file_count: 0,
            extracted_size: 0,
            warnings: Vec::new(),
            manifest: None,
        }
    }

    /// Add a warning to the validation result
    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    /// Mark validation as successful
    pub fn mark_valid(&mut self) {
        self.is_valid = true;
    }
}

/// Security policy for validation
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Allow setuid/setgid files
    pub allow_setuid: bool,
    /// Allow symlinks
    pub allow_symlinks: bool,
    /// Maximum allowed path depth
    pub max_path_depth: usize,
    /// Blocked file extensions
    pub blocked_extensions: Vec<String>,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            allow_setuid: false,
            allow_symlinks: true,
            max_path_depth: 100,
            blocked_extensions: vec![
                "exe".to_string(),
                "bat".to_string(),
                "cmd".to_string(),
                "scr".to_string(),
            ],
        }
    }
}

/// Validation context containing configuration and state
#[derive(Debug, Clone)]
pub struct ValidationContext {
    /// Security policy to apply
    pub security_policy: SecurityPolicy,
    /// Enable detailed content inspection
    pub detailed_inspection: bool,
    /// Maximum time to spend on validation (seconds)
    pub timeout_seconds: u64,
}

impl Default for ValidationContext {
    fn default() -> Self {
        Self {
            security_policy: SecurityPolicy::default(),
            detailed_inspection: true,
            timeout_seconds: 300, // 5 minutes
        }
    }
}

impl ValidationContext {
    /// Create a new validation context
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set security policy
    #[must_use]
    pub fn with_security_policy(mut self, policy: SecurityPolicy) -> Self {
        self.security_policy = policy;
        self
    }

    /// Enable or disable detailed inspection
    #[must_use]
    pub fn with_detailed_inspection(mut self, enabled: bool) -> Self {
        self.detailed_inspection = enabled;
        self
    }

    /// Set validation timeout
    #[must_use]
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = seconds;
        self
    }
}
