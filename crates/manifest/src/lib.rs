#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Package manifest handling for spsv2
//!
//! This crate defines the manifest.toml format and provides
//! serialization/deserialization for package metadata.

use serde::{Deserialize, Serialize};
use spsv2_errors::{Error, PackageError};
use spsv2_hash::Hash;
use spsv2_types::{Arch, Version, package::PackageSpec};
use std::path::Path;

/// Package manifest (manifest.toml contents)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub package: PackageInfo,
    pub dependencies: Dependencies,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sbom: Option<SbomInfo>,
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
    pub spdx: String, // SHA256 hash
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyclonedx: Option<String>, // SHA256 hash
}

impl Manifest {
    /// Create a new manifest
    pub fn new(name: String, version: Version, revision: u32, arch: Arch) -> Self {
        Self {
            package: PackageInfo {
                name,
                version: version.to_string(),
                revision,
                arch: arch.to_string(),
                description: None,
                homepage: None,
                license: None,
            },
            dependencies: Dependencies::default(),
            sbom: None,
        }
    }

    /// Parse the package version
    pub fn version(&self) -> Result<Version, Error> {
        Version::parse(&self.package.version).map_err(|_e| {
            spsv2_errors::VersionError::InvalidVersion {
                input: self.package.version.clone(),
            }
            .into()
        })
    }

    /// Parse the architecture
    pub fn arch(&self) -> Result<Arch, Error> {
        match self.package.arch.as_str() {
            "arm64" => Ok(Arch::Arm64),
            _ => Err(PackageError::InvalidFormat {
                message: format!("unsupported architecture: {}", self.package.arch),
            }
            .into()),
        }
    }

    /// Get runtime dependencies as PackageSpec
    pub fn runtime_deps(&self) -> Result<Vec<PackageSpec>, Error> {
        self.dependencies
            .runtime
            .iter()
            .map(|s| PackageSpec::parse(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Get build dependencies as PackageSpec
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
    pub fn set_sbom(&mut self, spdx_hash: Hash, cyclonedx_hash: Option<Hash>) {
        self.sbom = Some(SbomInfo {
            spdx: spdx_hash.to_hex(),
            cyclonedx: cyclonedx_hash.map(|h| h.to_hex()),
        });
    }

    /// Load manifest from TOML string
    pub fn from_toml(content: &str) -> Result<Self, Error> {
        toml::from_str(content).map_err(|e| {
            PackageError::InvalidManifest {
                message: e.to_string(),
            }
            .into()
        })
    }

    /// Load manifest from file
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
    pub fn to_toml(&self) -> Result<String, Error> {
        toml::to_string_pretty(self).map_err(|e| {
            PackageError::InvalidManifest {
                message: e.to_string(),
            }
            .into()
        })
    }

    /// Write manifest to file
    pub async fn write_to_file(&self, path: &Path) -> Result<(), Error> {
        let content = self.to_toml()?;
        tokio::fs::write(path, content).await.map_err(|e| {
            PackageError::InvalidManifest {
                message: format!("failed to write manifest: {e}"),
            }
            .into()
        })
    }

    /// Validate manifest fields
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

        Ok(())
    }

    /// Get package filename
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
    pub fn new(name: String, version: Version, arch: Arch) -> Self {
        Self {
            manifest: Manifest::new(name, version, 1, arch),
        }
    }

    /// Set revision
    pub fn revision(mut self, revision: u32) -> Self {
        self.manifest.package.revision = revision;
        self
    }

    /// Set description
    pub fn description(mut self, desc: String) -> Self {
        self.manifest.package.description = Some(desc);
        self
    }

    /// Set homepage
    pub fn homepage(mut self, url: String) -> Self {
        self.manifest.package.homepage = Some(url);
        self
    }

    /// Set license
    pub fn license(mut self, license: String) -> Self {
        self.manifest.package.license = Some(license);
        self
    }

    /// Add runtime dependency
    pub fn depends_on(mut self, spec: &str) -> Self {
        self.manifest.add_runtime_dep(spec);
        self
    }

    /// Add build dependency
    pub fn build_depends_on(mut self, spec: &str) -> Self {
        self.manifest.add_build_dep(spec);
        self
    }

    /// Build the manifest
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
            Version::parse("1.2.3").unwrap(),
            1,
            Arch::Arm64,
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
            Version::parse("1.7.0").unwrap(),
            1,
            Arch::Arm64,
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
            Version::parse("8.0.0").unwrap(),
            Arch::Arm64,
        )
        .description("Command line HTTP client".to_string())
        .homepage("https://curl.se".to_string())
        .license("MIT".to_string())
        .depends_on("openssl>=3.0.0")
        .depends_on("zlib~=1.2.0")
        .build_depends_on("pkg-config>=0.29")
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
            package: PackageInfo {
                name: String::new(), // Invalid: empty name
                version: "1.0.0".to_string(),
                revision: 1,
                arch: "arm64".to_string(),
                description: None,
                homepage: None,
                license: None,
            },
            dependencies: Dependencies::default(),
            sbom: None,
        };

        assert!(manifest.validate().is_err());
    }

    #[test]
    fn test_filename_generation() {
        let manifest = Manifest::new(
            "vim".to_string(),
            Version::parse("9.0.0").unwrap(),
            2,
            Arch::Arm64,
        );

        assert_eq!(manifest.filename(), "vim-9.0.0-2.arm64.sp");
    }
}
