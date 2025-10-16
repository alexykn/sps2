//! Package format versioning for sps2 package evolution
//!
//! This module provides comprehensive versioning support for the .sp package format,
//! enabling safe evolution and migration of the package format over time.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Package format version using semantic versioning
///
/// The package format version follows semantic versioning principles:
/// - Major: Breaking changes requiring migration (incompatible format changes)
/// - Minor: Backwards-compatible feature additions (new optional fields, compression types)
/// - Patch: Bug fixes and optimizations (no format changes)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PackageFormatVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl PackageFormatVersion {
    /// Current stable format version (v1.0.0)
    pub const CURRENT: Self = Self {
        major: 1,
        minor: 0,
        patch: 0,
    };

    /// Create a new package format version
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse version from string in format "major.minor.patch"
    ///
    /// # Errors
    ///
    /// Returns an error if the version string is malformed or contains invalid numbers
    pub fn parse(version_str: &str) -> Result<Self, PackageFormatVersionError> {
        let parts: Vec<&str> = version_str.split('.').collect();
        if parts.len() != 3 {
            return Err(PackageFormatVersionError::InvalidFormat {
                input: version_str.to_string(),
                reason: "Expected format: major.minor.patch".to_string(),
            });
        }

        let major =
            parts[0]
                .parse::<u32>()
                .map_err(|_| PackageFormatVersionError::InvalidNumber {
                    component: "major".to_string(),
                    value: parts[0].to_string(),
                })?;

        let minor =
            parts[1]
                .parse::<u32>()
                .map_err(|_| PackageFormatVersionError::InvalidNumber {
                    component: "minor".to_string(),
                    value: parts[1].to_string(),
                })?;

        let patch =
            parts[2]
                .parse::<u32>()
                .map_err(|_| PackageFormatVersionError::InvalidNumber {
                    component: "patch".to_string(),
                    value: parts[2].to_string(),
                })?;

        Ok(Self::new(major, minor, patch))
    }

    /// Check if this version is compatible with another version
    ///
    /// Compatibility rules:
    /// - Same major version: compatible
    /// - Different major version: incompatible (breaking changes)
    /// - Minor/patch differences within same major: compatible
    #[must_use]
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.major == other.major
    }

    /// Check if this version is newer than another
    #[must_use]
    pub fn is_newer_than(&self, other: &Self) -> bool {
        self > other
    }

    /// Check if this version requires migration from another version
    #[must_use]
    pub fn requires_migration_from(&self, other: &Self) -> bool {
        self.major != other.major
    }

    /// Get the compatibility matrix entry for this version
    #[must_use]
    pub fn compatibility_info(&self) -> PackageFormatCompatibility {
        match (self.major, self.minor, self.patch) {
            (1, 0, 0) => PackageFormatCompatibility {
                version: self.clone(),
                minimum_reader_version: Self::new(1, 0, 0),
                maximum_reader_version: Self::new(1, u32::MAX, u32::MAX),
                supports_signatures: true,
                deprecation_warning: None,
            },
            // Future versions would be added here
            _ => PackageFormatCompatibility {
                version: self.clone(),
                minimum_reader_version: self.clone(),
                maximum_reader_version: self.clone(),
                supports_signatures: true,
                deprecation_warning: Some(format!(
                    "Format version {self} is not officially supported"
                )),
            },
        }
    }

    /// Get version information for storage in package headers
    ///
    /// # Errors
    ///
    /// Returns an error if the `minor` or `patch` versions cannot fit into
    /// `u16` for header encoding.
    pub fn to_header_bytes(&self) -> Result<[u8; 12], PackageFormatVersionError> {
        let mut bytes = [0u8; 12];
        // Magic bytes for versioned package format: "SPV1" (0x53505631)
        bytes[0..4].copy_from_slice(&[0x53, 0x50, 0x56, 0x31]);
        // Major version (4 bytes, little endian)
        bytes[4..8].copy_from_slice(&self.major.to_le_bytes());
        // Minor version (2 bytes, little endian)
        let minor_u16 =
            u16::try_from(self.minor).map_err(|_| PackageFormatVersionError::InvalidNumber {
                component: "minor".to_string(),
                value: self.minor.to_string(),
            })?;
        bytes[8..10].copy_from_slice(&minor_u16.to_le_bytes());
        // Patch version (2 bytes, little endian)
        let patch_u16 =
            u16::try_from(self.patch).map_err(|_| PackageFormatVersionError::InvalidNumber {
                component: "patch".to_string(),
                value: self.patch.to_string(),
            })?;
        bytes[10..12].copy_from_slice(&patch_u16.to_le_bytes());
        Ok(bytes)
    }

    /// Parse version from package header bytes
    ///
    /// # Errors
    ///
    /// Returns an error if the header format is invalid or contains unsupported version
    pub fn from_header_bytes(bytes: &[u8]) -> Result<Self, PackageFormatVersionError> {
        if bytes.len() < 12 {
            return Err(PackageFormatVersionError::InvalidHeader {
                reason: "Header too short".to_string(),
            });
        }

        // Check magic bytes
        if bytes[0..4] != [0x53, 0x50, 0x56, 0x31] {
            return Err(PackageFormatVersionError::InvalidHeader {
                reason: "Invalid magic bytes in version header".to_string(),
            });
        }

        // Parse version components
        let major = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let minor = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        let patch = u32::from(u16::from_le_bytes([bytes[10], bytes[11]]));

        Ok(Self::new(major, minor, patch))
    }
}

impl fmt::Display for PackageFormatVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Default for PackageFormatVersion {
    fn default() -> Self {
        Self::CURRENT
    }
}

/// Package format compatibility information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageFormatCompatibility {
    /// The format version this compatibility info describes
    pub version: PackageFormatVersion,
    /// Minimum version of sps2 that can read this format
    pub minimum_reader_version: PackageFormatVersion,
    /// Maximum version of sps2 that can read this format
    pub maximum_reader_version: PackageFormatVersion,
    /// Whether this version supports package signatures
    pub supports_signatures: bool,
    /// Optional deprecation warning message
    pub deprecation_warning: Option<String>,
}

/// Migration information for upgrading packages between format versions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageFormatMigration {
    /// Source format version
    pub from_version: PackageFormatVersion,
    /// Target format version
    pub to_version: PackageFormatVersion,
    /// Whether automatic migration is possible
    pub automatic: bool,
    /// Migration steps required
    pub steps: Vec<MigrationStep>,
    /// Estimated time for migration
    pub estimated_duration: MigrationDuration,
}

/// Individual migration step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStep {
    /// Description of this migration step
    pub description: String,
    /// Whether this step is reversible
    pub reversible: bool,
    /// Data that might be lost in this step
    pub data_loss_warning: Option<String>,
}

/// Estimated duration for migration operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MigrationDuration {
    /// Migration completes instantly
    Instant,
    /// Migration takes seconds
    Seconds(u32),
    /// Migration takes minutes
    Minutes(u32),
    /// Migration takes hours
    Hours(u32),
}

/// Package format version validation result
#[derive(Debug, Clone)]
pub enum PackageFormatValidationResult {
    /// Format is compatible and can be processed
    Compatible,
    /// Format is newer but backwards compatible
    BackwardsCompatible {
        /// Warning message about newer format
        warning: String,
    },
    /// Format requires migration to be processed
    RequiresMigration {
        /// Available migration path
        migration: PackageFormatMigration,
    },
    /// Format is incompatible and cannot be processed
    Incompatible {
        /// Reason for incompatibility
        reason: String,
        /// Suggested action for user
        suggestion: String,
    },
}

/// Errors related to package format versioning
#[derive(Debug, Clone, thiserror::Error)]
pub enum PackageFormatVersionError {
    #[error("Invalid version format: {input} - {reason}")]
    InvalidFormat { input: String, reason: String },

    #[error("Invalid version number in {component}: {value}")]
    InvalidNumber { component: String, value: String },

    #[error("Invalid package header: {reason}")]
    InvalidHeader { reason: String },

    #[error("Unsupported format version: {version}")]
    UnsupportedVersion { version: String },

    #[error("Format version {version} requires migration from {current_version}")]
    MigrationRequired {
        version: String,
        current_version: String,
    },

    #[error("Format version {version} is incompatible with current reader")]
    IncompatibleVersion { version: String },
}

/// Package format version compatibility checker
#[derive(Clone, Debug)]
pub struct PackageFormatChecker {
    /// Current version this checker supports
    current_version: PackageFormatVersion,
}

impl PackageFormatChecker {
    /// Create a new format checker for the current version
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_version: PackageFormatVersion::CURRENT,
        }
    }

    /// Create a format checker for a specific version
    #[must_use]
    pub fn for_version(version: PackageFormatVersion) -> Self {
        Self {
            current_version: version,
        }
    }

    /// Validate a package format version for compatibility
    #[must_use]
    pub fn validate_version(
        &self,
        package_version: &PackageFormatVersion,
    ) -> PackageFormatValidationResult {
        if package_version == &self.current_version {
            return PackageFormatValidationResult::Compatible;
        }

        if package_version.is_compatible_with(&self.current_version) {
            if package_version.is_newer_than(&self.current_version) {
                PackageFormatValidationResult::BackwardsCompatible {
                    warning: format!(
                        "Package uses newer format version {} (current: {})",
                        package_version, self.current_version
                    ),
                }
            } else {
                PackageFormatValidationResult::Compatible
            }
        } else if package_version.requires_migration_from(&self.current_version) {
            let migration = self.get_migration_path(package_version);
            PackageFormatValidationResult::RequiresMigration { migration }
        } else {
            PackageFormatValidationResult::Incompatible {
                reason: format!(
                    "Format version {} is incompatible with current version {}",
                    package_version, self.current_version
                ),
                suggestion: "Upgrade sps2 to a newer version that supports this format".to_string(),
            }
        }
    }

    /// Get migration path from one version to another
    fn get_migration_path(&self, from_version: &PackageFormatVersion) -> PackageFormatMigration {
        // For now, provide a simple migration path
        // In the future, this would include more sophisticated migration logic
        PackageFormatMigration {
            from_version: from_version.clone(),
            to_version: self.current_version.clone(),
            automatic: from_version.major == self.current_version.major,
            steps: vec![MigrationStep {
                description: format!(
                    "Convert package from format {} to {}",
                    from_version, self.current_version
                ),
                reversible: false,
                data_loss_warning: None,
            }],
            estimated_duration: MigrationDuration::Seconds(30),
        }
    }

    /// Get all migration paths available from a version
    #[must_use]
    pub fn available_migrations(
        &self,
        from_version: &PackageFormatVersion,
    ) -> Vec<PackageFormatMigration> {
        // For now, only support migration to current version
        // Future implementations could support migration to multiple target versions
        vec![self.get_migration_path(from_version)]
    }
}

impl Default for PackageFormatChecker {
    fn default() -> Self {
        Self::new()
    }
}
