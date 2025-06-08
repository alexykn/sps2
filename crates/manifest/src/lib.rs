#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Package manifest handling for sps2
//!
//! This crate defines the manifest.toml format and provides
//! serialization/deserialization for package metadata.

use serde::{Deserialize, Serialize};
use sps2_errors::{Error, PackageError};
use sps2_hash::Hash;
use sps2_types::{
    format::CompressionFormatType, package::PackageSpec, Arch, PackageFormatVersion,
    PythonPackageMetadata, Version,
};
use std::path::Path;

/// Package manifest (manifest.toml contents)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Package format version for compatibility checking
    #[serde(default = "PackageFormatVersion::default")]
    pub format_version: PackageFormatVersion,
    pub package: PackageInfo,
    pub dependencies: Dependencies,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression: Option<CompressionInfo>,
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
    pub spdx: String, // BLAKE3 hash
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyclonedx: Option<String>, // BLAKE3 hash
}

/// Compression information section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionInfo {
    /// Compression format type
    pub format: CompressionFormatType,
    /// Frame size for seekable compression (in bytes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_size: Option<usize>,
    /// Number of frames (seekable format only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_count: Option<usize>,
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
                compression: None,
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

    /// Set SBOM hashes
    pub fn set_sbom(&mut self, spdx_hash: &Hash, cyclonedx_hash: Option<&Hash>) {
        self.sbom = Some(SbomInfo {
            spdx: spdx_hash.to_hex(),
            cyclonedx: cyclonedx_hash.map(sps2_hash::Hash::to_hex),
        });
    }

    /// Set compression information for legacy format
    pub fn set_compression_legacy(&mut self) {
        self.package.compression = Some(CompressionInfo {
            format: CompressionFormatType::Legacy,
            frame_size: None,
            frame_count: None,
        });
    }

    /// Set compression information for seekable format
    pub fn set_compression_seekable(&mut self, frame_size: usize, frame_count: Option<usize>) {
        self.package.compression = Some(CompressionInfo {
            format: CompressionFormatType::Seekable,
            frame_size: Some(frame_size),
            frame_count,
        });
    }

    /// Set Python package metadata
    pub fn set_python_metadata(&mut self, metadata: PythonPackageMetadata) {
        self.python = Some(metadata);
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

    /// Load manifest from file
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if the TOML content is malformed.
    pub async fn from_file(path: &Path) -> Result<Self, Error> {
        let content =
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| PackageError::InvalidManifest {
                    message: format!("failed to read manifest: {e}"),
                })?;
        Self::from_toml(&content)
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

    /// Write manifest to file
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest cannot be serialized or if the file cannot be written.
    pub async fn write_to_file(&self, path: &Path) -> Result<(), Error> {
        let content = self.to_toml()?;
        tokio::fs::write(path, content).await.map_err(|e| {
            PackageError::InvalidManifest {
                message: format!("failed to write manifest: {e}"),
            }
            .into()
        })
    }

    /// Get the package format version
    #[must_use]
    pub fn format_version(&self) -> &PackageFormatVersion {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_creation() {
        let manifest = Manifest::new(
            "test-pkg".to_string(),
            &Version::parse("1.2.3").unwrap(),
            1,
            &Arch::Arm64,
        );

        assert_eq!(manifest.package.name, "test-pkg");
        assert_eq!(manifest.package.version, "1.2.3");
        assert_eq!(manifest.package.revision, 1);
        assert_eq!(manifest.package.arch, "arm64");
    }

    #[test]
    fn test_manifest_toml_roundtrip() {
        let mut manifest = Manifest::new(
            "jq".to_string(),
            &Version::parse("1.7.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        manifest.add_runtime_dep("oniguruma==6.9.8");
        manifest.add_build_dep("autoconf>=2.71");

        let toml = manifest.to_toml().unwrap();
        let parsed = Manifest::from_toml(&toml).unwrap();

        assert_eq!(parsed.package.name, manifest.package.name);
        assert_eq!(parsed.dependencies.runtime, manifest.dependencies.runtime);
        assert_eq!(parsed.dependencies.build, manifest.dependencies.build);
    }

    #[test]
    fn test_manifest_builder() {
        let manifest = ManifestBuilder::new(
            "curl".to_string(),
            &Version::parse("8.0.0").unwrap(),
            &Arch::Arm64,
        )
        .description("Command line HTTP client".to_string())
        .homepage("https://curl.se".to_string())
        .license("MIT".to_string())
        .depends_on("openssl>=3.0.0")
        .depends_on("zlib~=1.2.0")
        .build_depends_on("pkg-config>=0.29.0")
        .build()
        .unwrap();

        assert_eq!(
            manifest.package.description.as_deref(),
            Some("Command line HTTP client")
        );
        assert_eq!(manifest.dependencies.runtime.len(), 2);
        assert_eq!(manifest.dependencies.build.len(), 1);
    }

    #[test]
    fn test_manifest_validation() {
        let manifest = Manifest {
            format_version: PackageFormatVersion::CURRENT,
            package: PackageInfo {
                name: String::new(), // Invalid: empty name
                version: "1.0.0".to_string(),
                revision: 1,
                arch: "arm64".to_string(),
                description: None,
                homepage: None,
                license: None,
                compression: None,
            },
            dependencies: Dependencies::default(),
            sbom: None,
            python: None,
        };

        assert!(manifest.validate().is_err());
    }

    #[test]
    fn test_filename_generation() {
        let manifest = Manifest::new(
            "vim".to_string(),
            &Version::parse("9.0.0").unwrap(),
            2,
            &Arch::Arm64,
        );

        assert_eq!(manifest.filename(), "vim-9.0.0-2.arm64.sp");
    }

    #[test]
    fn test_compression_info_legacy() {
        let mut manifest = Manifest::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        manifest.set_compression_legacy();

        assert!(manifest.package.compression.is_some());
        let compression = manifest.package.compression.unwrap();
        assert_eq!(compression.format, CompressionFormatType::Legacy);
        assert!(compression.frame_size.is_none());
        assert!(compression.frame_count.is_none());
    }

    #[test]
    fn test_compression_info_seekable() {
        let mut manifest = Manifest::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        manifest.set_compression_seekable(1024 * 1024, Some(5));

        assert!(manifest.package.compression.is_some());
        let compression = manifest.package.compression.unwrap();
        assert_eq!(compression.format, CompressionFormatType::Seekable);
        assert_eq!(compression.frame_size, Some(1024 * 1024));
        assert_eq!(compression.frame_count, Some(5));
    }

    #[test]
    fn test_compression_info_toml_serialization() {
        let mut manifest = Manifest::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        manifest.set_compression_seekable(1024 * 1024, Some(3));

        let toml = manifest.to_toml().unwrap();
        let parsed = Manifest::from_toml(&toml).unwrap();

        assert_eq!(
            parsed.package.compression.as_ref().unwrap().format,
            CompressionFormatType::Seekable
        );
        assert_eq!(
            parsed.package.compression.as_ref().unwrap().frame_size,
            Some(1024 * 1024)
        );
        assert_eq!(
            parsed.package.compression.as_ref().unwrap().frame_count,
            Some(3)
        );
    }

    #[test]
    fn test_format_version_default() {
        let manifest = Manifest::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        assert_eq!(manifest.format_version, PackageFormatVersion::CURRENT);
        assert_eq!(manifest.format_version().major, 1);
        assert_eq!(manifest.format_version().minor, 0);
        assert_eq!(manifest.format_version().patch, 0);
    }

    #[test]
    fn test_format_version_compatibility() {
        let manifest = Manifest::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        // Same version should be compatible
        assert!(manifest.is_compatible_with(&PackageFormatVersion::new(1, 0, 0)));

        // Different minor/patch within same major should be compatible
        assert!(manifest.is_compatible_with(&PackageFormatVersion::new(1, 1, 0)));
        assert!(manifest.is_compatible_with(&PackageFormatVersion::new(1, 0, 1)));

        // Different major version should be incompatible
        assert!(!manifest.is_compatible_with(&PackageFormatVersion::new(2, 0, 0)));
    }

    #[test]
    fn test_format_version_migration_requirements() {
        let manifest = Manifest::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        // Same major version should not require migration
        assert!(!manifest.requires_migration_to(&PackageFormatVersion::new(1, 1, 0)));

        // Different major version should require migration
        assert!(manifest.requires_migration_to(&PackageFormatVersion::new(2, 0, 0)));
    }

    #[test]
    fn test_format_version_setting() {
        let mut manifest = Manifest::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        let new_version = PackageFormatVersion::new(1, 1, 0);
        manifest.set_format_version(new_version.clone());
        assert_eq!(manifest.format_version, new_version);
    }

    #[test]
    fn test_format_version_builder() {
        let manifest = ManifestBuilder::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            &Arch::Arm64,
        )
        .format_version(PackageFormatVersion::new(1, 2, 3))
        .build()
        .unwrap();

        assert_eq!(manifest.format_version, PackageFormatVersion::new(1, 2, 3));
    }

    #[test]
    fn test_format_version_toml_serialization() {
        let manifest = Manifest::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        let toml = manifest.to_toml().unwrap();
        let parsed = Manifest::from_toml(&toml).unwrap();

        assert_eq!(parsed.format_version, manifest.format_version);
        assert_eq!(parsed.format_version, PackageFormatVersion::CURRENT);
    }

    #[test]
    fn test_format_version_validation_incompatible() {
        let mut manifest = Manifest::new(
            "test".to_string(),
            &Version::parse("1.0.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        // Set an incompatible version (major version 999)
        manifest.set_format_version(PackageFormatVersion::new(999, 0, 0));

        // Validation should fail
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn test_python_metadata() {
        use std::collections::HashMap;

        let mut manifest = Manifest::new(
            "black".to_string(),
            &Version::parse("23.1.0").unwrap(),
            1,
            &Arch::Arm64,
        );

        // Initially no Python metadata
        assert!(manifest.python.is_none());

        // Create Python metadata
        let mut executables = HashMap::new();
        executables.insert("black".to_string(), "black:main".to_string());
        executables.insert("blackd".to_string(), "blackd:main".to_string());

        let python_meta = PythonPackageMetadata {
            requires_python: ">=3.8,<3.12".to_string(),
            wheel_file: "python/black-23.1.0-py3-none-any.whl".to_string(),
            requirements_file: "python/requirements.lock.txt".to_string(),
            executables,
        };

        manifest.set_python_metadata(python_meta.clone());
        assert!(manifest.python.is_some());
        assert_eq!(
            manifest.python.as_ref().unwrap().requires_python,
            ">=3.8,<3.12"
        );

        // Test TOML roundtrip with Python metadata
        let toml = manifest.to_toml().unwrap();
        let parsed = Manifest::from_toml(&toml).unwrap();

        assert!(parsed.python.is_some());
        let parsed_python = parsed.python.unwrap();
        assert_eq!(parsed_python.requires_python, ">=3.8,<3.12");
        assert_eq!(
            parsed_python.wheel_file,
            "python/black-23.1.0-py3-none-any.whl"
        );
        assert_eq!(parsed_python.executables.len(), 2);
        assert_eq!(
            parsed_python.executables.get("black"),
            Some(&"black:main".to_string())
        );
    }

    #[test]
    fn test_python_metadata_builder() {
        use std::collections::HashMap;

        let mut executables = HashMap::new();
        executables.insert("myapp".to_string(), "myapp.cli:main".to_string());

        let python_meta = PythonPackageMetadata {
            requires_python: ">=3.9".to_string(),
            wheel_file: "python/myapp-1.0.0-py3-none-any.whl".to_string(),
            requirements_file: "python/requirements.lock.txt".to_string(),
            executables,
        };

        let manifest = ManifestBuilder::new(
            "myapp".to_string(),
            &Version::parse("1.0.0").unwrap(),
            &Arch::Arm64,
        )
        .description("My Python app".to_string())
        .python_metadata(python_meta)
        .build()
        .unwrap();

        assert!(manifest.python.is_some());
        assert_eq!(manifest.python.as_ref().unwrap().requires_python, ">=3.9");
    }
}
