#![allow(clippy::module_name_repetitions)]

//! Package manifest handling types for sps2
//!
//! This module defines the `manifest.toml` format and provides
//! serialization/deserialization and validation for package metadata.

use crate::{package::PackageSpec, Arch, PackageFormatVersion, PythonPackageMetadata, Version};
use serde::{de::IgnoredAny, Deserialize, Serialize};
use sps2_errors::{Error, PackageError};

/// Package manifest (manifest.toml contents)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Package format version for compatibility checking
    #[serde(default = "PackageFormatVersion::default")]
    pub format_version: PackageFormatVersion,
    pub package: PackageInfo,
    pub dependencies: Dependencies,
    /// SBOM may be absent while SBOM is soft-disabled
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sbom: Option<SbomInfo>,
    /// Optional Python-specific metadata for Python packages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python: Option<PythonPackageMetadata>,
}

/// Package information section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub revision: u32,
    pub arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Legacy compression configuration retained for backward compatibility
    #[serde(default, alias = "compression", skip_serializing)]
    pub legacy_compression: Option<IgnoredAny>,
}

/// Dependencies section
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dependencies {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build: Vec<String>,
}

/// SBOM information section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbomInfo {
    pub spdx: String, // BLAKE3 hash (hex)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyclonedx: Option<String>, // BLAKE3 hash (hex)
}

impl Manifest {
    /// Create a new manifest
    #[must_use]
    pub fn new(name: String, version: &Version, revision: u32, arch: &Arch) -> Self {
        Self {
            format_version: PackageFormatVersion::CURRENT,
            package: PackageInfo {
                name,
                version: version.to_string(),
                revision,
                arch: arch.to_string(),
                description: None,
                homepage: None,
                license: None,
                legacy_compression: None,
            },
            dependencies: Dependencies::default(),
            sbom: None,
            python: None,
        }
    }

    /// Parse the package version
    ///
    /// # Errors
    ///
    /// Returns an error if the version string is not a valid semantic version.
    pub fn version(&self) -> Result<Version, Error> {
        Version::parse(&self.package.version).map_err(|_e| {
            sps2_errors::VersionError::InvalidVersion {
                input: self.package.version.clone(),
            }
            .into()
        })
    }

    /// Parse the architecture
    ///
    /// # Errors
    ///
    /// Returns an error if the architecture string is not supported (currently only "arm64" is supported).
    pub fn arch(&self) -> Result<Arch, Error> {
        match self.package.arch.as_str() {
            "arm64" => Ok(Arch::Arm64),
            _ => Err(PackageError::InvalidFormat {
                message: format!("unsupported architecture: {}", self.package.arch),
            }
            .into()),
        }
    }

    /// Get runtime dependencies as `PackageSpec`
    ///
    /// # Errors
    ///
    /// Returns an error if any dependency specification string is invalid or cannot be parsed.
    pub fn runtime_deps(&self) -> Result<Vec<PackageSpec>, Error> {
        self.dependencies
            .runtime
            .iter()
            .map(|s| PackageSpec::parse(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Get build dependencies as `PackageSpec`
    ///
    /// # Errors
    ///
    /// Returns an error if any dependency specification string is invalid or cannot be parsed.
    pub fn build_deps(&self) -> Result<Vec<PackageSpec>, Error> {
        self.dependencies
            .build
            .iter()
            .map(|s| PackageSpec::parse(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Add a runtime dependency
    pub fn add_runtime_dep(&mut self, spec: &str) {
        self.dependencies.runtime.push(spec.to_string());
    }

    /// Add a build dependency
    pub fn add_build_dep(&mut self, spec: &str) {
        self.dependencies.build.push(spec.to_string());
    }

    /// Serialize to TOML string
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest cannot be serialized to TOML format.
    pub fn to_toml(&self) -> Result<String, Error> {
        toml::to_string_pretty(self).map_err(|e| {
            PackageError::InvalidManifest {
                message: e.to_string(),
            }
            .into()
        })
    }

    /// Load manifest from TOML string
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML content is malformed or contains invalid manifest data.
    pub fn from_toml(content: &str) -> Result<Self, Error> {
        toml::from_str(content).map_err(|e| {
            PackageError::InvalidManifest {
                message: e.to_string(),
            }
            .into()
        })
    }

    /// Get the package format version
    #[must_use]
    pub const fn format_version(&self) -> &PackageFormatVersion {
        &self.format_version
    }

    /// Set the package format version
    pub fn set_format_version(&mut self, version: PackageFormatVersion) {
        self.format_version = version;
    }

    /// Check if this manifest is compatible with a specific format version
    #[must_use]
    pub fn is_compatible_with(&self, other_version: &PackageFormatVersion) -> bool {
        self.format_version.is_compatible_with(other_version)
    }

    /// Check if this manifest requires migration to be compatible with a version
    #[must_use]
    pub fn requires_migration_to(&self, target_version: &PackageFormatVersion) -> bool {
        self.format_version.requires_migration_from(target_version)
    }

    /// Validate manifest fields
    ///
    /// # Errors
    ///
    /// Returns an error if any required field is empty, invalid, or if dependency specifications are malformed.
    pub fn validate(&self) -> Result<(), Error> {
        // Validate name
        if self.package.name.is_empty() {
            return Err(PackageError::InvalidManifest {
                message: "package name cannot be empty".to_string(),
            }
            .into());
        }

        // Validate version
        self.version()?;

        // Validate arch
        self.arch()?;

        // Validate dependencies
        self.runtime_deps()?;
        self.build_deps()?;

        // Validate format version compatibility
        let current_version = PackageFormatVersion::CURRENT;
        if !self.format_version.is_compatible_with(&current_version) {
            return Err(PackageError::InvalidManifest {
                message: format!(
                    "Package format version {} is incompatible with current version {}",
                    self.format_version, current_version
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Get package filename
    #[must_use]
    pub fn filename(&self) -> String {
        format!(
            "{}-{}-{}.{}.sp",
            self.package.name, self.package.version, self.package.revision, self.package.arch
        )
    }
}

/// Builder for creating manifests
#[derive(Debug, Clone)]
pub struct ManifestBuilder {
    manifest: Manifest,
}

impl ManifestBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new(name: String, version: &Version, arch: &Arch) -> Self {
        Self {
            manifest: Manifest::new(name, version, 1, arch),
        }
    }

    /// Set package format version
    #[must_use]
    pub fn format_version(mut self, version: PackageFormatVersion) -> Self {
        self.manifest.format_version = version;
        self
    }

    /// Set revision
    #[must_use]
    pub fn revision(mut self, revision: u32) -> Self {
        self.manifest.package.revision = revision;
        self
    }

    /// Set description
    #[must_use]
    pub fn description(mut self, desc: String) -> Self {
        self.manifest.package.description = Some(desc);
        self
    }

    /// Set homepage
    #[must_use]
    pub fn homepage(mut self, url: String) -> Self {
        self.manifest.package.homepage = Some(url);
        self
    }

    /// Set license
    #[must_use]
    pub fn license(mut self, license: String) -> Self {
        self.manifest.package.license = Some(license);
        self
    }

    /// Add runtime dependency
    #[must_use]
    pub fn depends_on(mut self, spec: &str) -> Self {
        self.manifest.add_runtime_dep(spec);
        self
    }

    /// Add build dependency
    #[must_use]
    pub fn build_depends_on(mut self, spec: &str) -> Self {
        self.manifest.add_build_dep(spec);
        self
    }

    /// Set Python package metadata
    #[must_use]
    pub fn python_metadata(mut self, metadata: PythonPackageMetadata) -> Self {
        self.manifest.python = Some(metadata);
        self
    }

    /// Build the manifest
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest validation fails.
    pub fn build(self) -> Result<Manifest, Error> {
        self.manifest.validate()?;
        Ok(self.manifest)
    }
}
