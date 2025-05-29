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
    pub fn parse(s: &str) -> Result<Self, spsv2_errors::VersionError> {
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
            return Err(spsv2_errors::VersionError::InvalidConstraint {
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
    pub name: String,
    pub version: Version,
    pub description: Option<String>,
    pub installed: bool,
    pub size: u64,
    pub arch: Arch,
    pub dependencies: Vec<PackageSpec>,
}

/// Search result from package index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub name: String,
    pub version: Version,
    pub description: String,
    pub homepage: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_spec_parse() {
        let spec = PackageSpec::parse("jq>=1.6.0").unwrap();
        assert_eq!(spec.name, "jq");
        assert_eq!(spec.version_spec.to_string(), ">=1.6.0");

        let spec = PackageSpec::parse("curl").unwrap();
        assert_eq!(spec.name, "curl");
        assert!(spec.version_spec.is_any());

        let spec = PackageSpec::parse("openssl>=1.1.0,<2.0.0").unwrap();
        assert_eq!(spec.name, "openssl");
        assert_eq!(spec.version_spec.to_string(), ">=1.1.0,<2.0.0");
    }

    #[test]
    fn test_package_id_display() {
        let id = PackageId::new("jq", Version::parse("1.7.0").unwrap());
        assert_eq!(id.to_string(), "jq-1.7.0");
    }
}
