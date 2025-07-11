//! Package-related type definitions

use crate::{Arch, Version, VersionSpec};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for a package
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageId {
    pub name: String,
    pub version: Version,
}

impl PackageId {
    /// Create a new package ID
    pub fn new(name: impl Into<String>, version: Version) -> Self {
        Self {
            name: name.into(),
            version,
        }
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.name, self.version)
    }
}

/// Package specification with optional version constraints
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSpec {
    pub name: String,
    pub version_spec: VersionSpec,
}

impl PackageSpec {
    /// Parse a package spec from a string (e.g., "jq>=1.6,<2.0")
    ///
    /// # Errors
    ///
    /// Returns `VersionError` if the package specification string is malformed
    /// or contains invalid version constraints.
    ///
    /// # Panics
    ///
    /// This function may panic if the input string contains malformed version
    /// constraints that cannot be parsed.
    pub fn parse(s: &str) -> Result<Self, sps2_errors::VersionError> {
        // Find the first constraint operator
        let operators = ["==", ">=", "<=", "!=", "~=", ">", "<"];
        let mut split_pos = None;

        for op in &operators {
            if let Some(pos) = s.find(op) {
                match split_pos {
                    None => split_pos = Some(pos),
                    Some(sp) if pos < sp => split_pos = Some(pos),
                    Some(_) => {}
                }
            }
        }

        let (name, version_str) = if let Some(pos) = split_pos {
            (s[..pos].trim(), s[pos..].trim())
        } else {
            // No version constraint means any version
            (s.trim(), "*")
        };

        if name.is_empty() {
            return Err(sps2_errors::VersionError::InvalidConstraint {
                input: s.to_string(),
            });
        }

        Ok(Self {
            name: name.to_string(),
            version_spec: version_str.parse()?,
        })
    }
}

impl fmt::Display for PackageSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.version_spec.is_any() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{}{}", self.name, self.version_spec)
        }
    }
}

/// Package information for installed packages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    /// Package name
    pub name: String,
    /// Installed version
    pub version: Option<Version>,
    /// Available version
    pub available_version: Option<Version>,
    /// Description
    pub description: Option<String>,
    /// Homepage URL
    pub homepage: Option<String>,
    /// License
    pub license: Option<String>,
    /// Installation status
    pub status: PackageStatus,
    /// Dependencies (as strings for simplicity)
    pub dependencies: Vec<String>,
    /// Size on disk (bytes)
    pub size: Option<u64>,
    /// Architecture
    pub arch: Option<Arch>,
    /// Whether package is installed
    pub installed: bool,
}

/// Package installation status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageStatus {
    /// Not installed
    Available,
    /// Installed and up to date
    Installed,
    /// Installed but update available
    Outdated,
    /// Installed from local file
    Local,
}

/// Search result from package index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Package name
    pub name: String,
    /// Latest version
    pub version: Version,
    /// Description
    pub description: Option<String>,
    /// Homepage URL
    pub homepage: Option<String>,
    /// Whether package is installed
    pub installed: bool,
}

/// Dependency kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DepKind {
    Build,
    Runtime,
}

impl fmt::Display for DepKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build => write!(f, "build"),
            Self::Runtime => write!(f, "runtime"),
        }
    }
}

/// Dependency edge in resolver graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepEdge {
    pub name: String,
    pub spec: VersionSpec,
    pub kind: DepKind,
}

/// Python-specific metadata for packages that use Python
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonPackageMetadata {
    /// The Python version requirement (e.g., ">=3.9,<3.12")
    pub requires_python: String,

    /// Path within the package to the built wheel file
    pub wheel_file: String,

    /// Path within the package to the locked requirements file
    pub requirements_file: String,

    /// Mapping of executable names to their Python entry points
    /// e.g., {"black": "black:main", "blackd": "blackd:main"}
    pub executables: std::collections::HashMap<String, String>,
}
