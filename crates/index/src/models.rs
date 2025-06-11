//! Index data models

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sps2_errors::{Error, PackageError};
use sps2_types::Arch;
use std::collections::HashMap;

/// Repository index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    #[serde(flatten)]
    pub metadata: IndexMetadata,
    pub packages: HashMap<String, PackageEntry>,
}

/// Index metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexMetadata {
    pub version: u32,
    pub minimum_client: String,
    pub timestamp: DateTime<Utc>,
}

/// Package entry in index
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageEntry {
    pub versions: HashMap<String, VersionEntry>,
}

/// Version entry in index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub revision: u32,
    pub arch: String,
    pub blake3: String,
    pub download_url: String,
    pub minisig_url: String,
    pub dependencies: DependencyInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sbom: Option<SbomInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

/// Dependency information
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyInfo {
    #[serde(default)]
    pub runtime: Vec<String>,
    #[serde(default)]
    pub build: Vec<String>,
}

/// SBOM information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbomInfo {
    pub spdx: SbomEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyclonedx: Option<SbomEntry>,
}

/// SBOM entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbomEntry {
    pub url: String,
    pub blake3: String,
}

impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}

impl Index {
    /// Create a new empty index
    #[must_use]
    pub fn new() -> Self {
        Self {
            metadata: IndexMetadata {
                version: crate::SUPPORTED_INDEX_VERSION,
                minimum_client: "0.1.0".to_string(),
                timestamp: Utc::now(),
            },
            packages: HashMap::new(),
        }
    }

    /// Parse index from JSON
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is malformed or cannot be parsed.
    pub fn from_json(json: &str) -> Result<Self, Error> {
        serde_json::from_str(json).map_err(|e| {
            PackageError::InvalidFormat {
                message: format!("invalid index JSON: {e}"),
            }
            .into()
        })
    }

    /// Serialize index to JSON
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot be serialized to JSON.
    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(self).map_err(|e| {
            PackageError::InvalidFormat {
                message: format!("failed to serialize index: {e}"),
            }
            .into()
        })
    }

    /// Validate index format and version
    ///
    /// # Errors
    ///
    /// Returns an error if the index version is unsupported, package names are empty,
    /// versions are missing, architectures are unsupported, or required fields are missing.
    pub fn validate(&self) -> Result<(), Error> {
        // Check version compatibility
        if self.metadata.version > crate::SUPPORTED_INDEX_VERSION {
            return Err(PackageError::InvalidFormat {
                message: format!(
                    "index version {} is newer than supported version {}",
                    self.metadata.version,
                    crate::SUPPORTED_INDEX_VERSION
                ),
            }
            .into());
        }

        // Validate entries
        for (name, package) in &self.packages {
            if name.is_empty() {
                return Err(PackageError::InvalidFormat {
                    message: "empty package name in index".to_string(),
                }
                .into());
            }

            for (version, entry) in &package.versions {
                if version.is_empty() {
                    return Err(PackageError::InvalidFormat {
                        message: format!("empty version for package {name}"),
                    }
                    .into());
                }

                // Validate architecture
                if entry.arch != "arm64" {
                    return Err(PackageError::InvalidFormat {
                        message: format!("unsupported architecture: {}", entry.arch),
                    }
                    .into());
                }

                // Validate URLs
                if entry.download_url.is_empty() {
                    return Err(PackageError::InvalidFormat {
                        message: format!("missing download URL for {name}-{version}"),
                    }
                    .into());
                }

                if entry.blake3.is_empty() {
                    return Err(PackageError::InvalidFormat {
                        message: format!("missing BLAKE3 hash for {name}-{version}"),
                    }
                    .into());
                }
            }
        }

        Ok(())
    }

    /// Add or update a package version
    pub fn add_version(&mut self, name: String, version: String, entry: VersionEntry) {
        self.packages
            .entry(name)
            .or_default()
            .versions
            .insert(version, entry);
    }

    /// Remove a package version
    pub fn remove_version(&mut self, name: &str, version: &str) -> Option<VersionEntry> {
        self.packages.get_mut(name)?.versions.remove(version)
    }

    /// Get total package count
    #[must_use]
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Get total version count
    #[must_use]
    pub fn version_count(&self) -> usize {
        self.packages.values().map(|p| p.versions.len()).sum()
    }
}

impl VersionEntry {
    /// Get the version string from the parent context
    /// (In actual use, version is the `HashMap` key)
    #[must_use]
    pub fn version(&self) -> String {
        // This is a placeholder - in practice, the version
        // is known from the HashMap key when accessing this entry
        String::new()
    }

    /// Get architecture as enum
    ///
    /// # Errors
    ///
    /// Returns an error if the architecture string is not supported.
    pub fn arch(&self) -> Result<Arch, Error> {
        match self.arch.as_str() {
            "arm64" => Ok(Arch::Arm64),
            _ => Err(PackageError::InvalidFormat {
                message: format!("unsupported architecture: {}", self.arch),
            }
            .into()),
        }
    }

    /// Check if this version has SBOM data
    #[must_use]
    pub fn has_sbom(&self) -> bool {
        self.sbom.is_some()
    }
}
