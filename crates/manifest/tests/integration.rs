//! Integration tests for manifest crate

#[cfg(test)]
mod tests {
    use spsv2_hash::Hash;
    use spsv2_manifest::*;
    use spsv2_types::{Arch, Version};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_manifest_file_operations() {
        let temp = tempdir().unwrap();
        let manifest_path = temp.path().join("manifest.toml");

        // Create manifest
        let mut manifest = Manifest::new(
            "test-app".to_string(),
            Version::parse("2.5.0").unwrap(),
            3,
            Arch::Arm64,
        );

        manifest.package.description = Some("Test application".to_string());
        manifest.add_runtime_dep("libfoo>=1.0.0");
        manifest.add_build_dep("make>=4.0");

        // Set SBOM info
        let spdx_hash = Hash::hash(b"spdx content");
        let cdx_hash = Hash::hash(b"cyclonedx content");
        manifest.set_sbom(spdx_hash, Some(cdx_hash));

        // Write to file
        manifest.write_to_file(&manifest_path).await.unwrap();

        // Read back
        let loaded = Manifest::from_file(&manifest_path).await.unwrap();

        // Verify
        assert_eq!(loaded.package.name, "test-app");
        assert_eq!(loaded.package.version, "2.5.0");
        assert_eq!(loaded.package.revision, 3);
        assert_eq!(
            loaded.package.description.as_deref(),
            Some("Test application")
        );
        assert_eq!(loaded.dependencies.runtime, vec!["libfoo>=1.0.0"]);
        assert_eq!(loaded.dependencies.build, vec!["make>=4.0"]);

        // Check SBOM
        assert!(loaded.sbom.is_some());
        let sbom = loaded.sbom.unwrap();
        assert_eq!(sbom.spdx, spdx_hash.to_hex());
        assert_eq!(sbom.cyclonedx.as_deref(), Some(&cdx_hash.to_hex()));
    }

    #[test]
    fn test_dependency_parsing() {
        let manifest = ManifestBuilder::new(
            "pkg".to_string(),
            Version::parse("1.0.0").unwrap(),
            Arch::Arm64,
        )
        .depends_on("dep1>=1.0.0,<2.0.0")
        .depends_on("dep2~=3.4.0")
        .build_depends_on("build-dep==1.2.3")
        .build()
        .unwrap();

        // Parse runtime deps
        let runtime_deps = manifest.runtime_deps().unwrap();
        assert_eq!(runtime_deps.len(), 2);
        assert_eq!(runtime_deps[0].name, "dep1");
        assert_eq!(runtime_deps[1].name, "dep2");

        // Parse build deps
        let build_deps = manifest.build_deps().unwrap();
        assert_eq!(build_deps.len(), 1);
        assert_eq!(build_deps[0].name, "build-dep");
        assert_eq!(build_deps[0].version_spec.to_string(), "==1.2.3");
    }

    #[test]
    fn test_manifest_from_toml_string() {
        let toml = r#"
[package]
name = "curl"
version = "8.5.0"
revision = 1
arch = "arm64"
description = "Command line HTTP client"
homepage = "https://curl.se"
license = "MIT"

[dependencies]
runtime = [
    "openssl>=3.0.0",
    "zlib~=1.2.0",
    "libidn2>=2.0.0"
]
build = [
    "pkg-config>=0.29",
    "perl>=5.0"
]

[sbom]
spdx = "abc123def456"
cyclonedx = "789xyz"
"#;

        let manifest = Manifest::from_toml(toml).unwrap();

        assert_eq!(manifest.package.name, "curl");
        assert_eq!(manifest.package.version, "8.5.0");
        assert_eq!(manifest.package.license.as_deref(), Some("MIT"));
        assert_eq!(manifest.dependencies.runtime.len(), 3);
        assert_eq!(manifest.dependencies.build.len(), 2);
        assert_eq!(manifest.sbom.as_ref().unwrap().spdx, "abc123def456");
    }

    #[test]
    fn test_invalid_manifest() {
        // Missing required fields
        let toml = r#"
[package]
name = "incomplete"
# Missing version, revision, arch
"#;

        assert!(Manifest::from_toml(toml).is_err());

        // Invalid dependency spec
        let mut manifest = Manifest::new(
            "bad-deps".to_string(),
            Version::parse("1.0.0").unwrap(),
            1,
            Arch::Arm64,
        );
        manifest.add_runtime_dep("invalid@@version");

        assert!(manifest.runtime_deps().is_err());
    }
}
