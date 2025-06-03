//! Parsing and validation integration tests

use super::common::TestEnvironment;
use sps2_hash::Hash;
use sps2_types::{PackageSpec, Version};
use tempfile::NamedTempFile;
use tokio::fs;

#[tokio::test]
async fn test_manifest_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnvironment::new().await?;

    // Test simple manifest
    let hello_manifest = env.load_test_manifest("hello-world-1.0.0").await?;
    let manifest = sps2_manifest::Manifest::from_toml(&hello_manifest)?;

    assert_eq!(manifest.package.name, "hello-world");
    assert_eq!(manifest.package.version, "1.0.0");
    assert_eq!(manifest.package.license, Some("MIT".to_string()));
    assert!(manifest.dependencies.runtime.is_empty());
    assert!(manifest.dependencies.build.is_empty());

    // Test complex manifest
    let complex_manifest = env.load_test_manifest("complex-app-2.1.3").await?;
    let manifest = sps2_manifest::Manifest::from_toml(&complex_manifest)?;

    assert_eq!(manifest.package.name, "complex-app");
    assert_eq!(manifest.package.version, "2.1.3");
    assert_eq!(manifest.package.license, Some("Apache-2.0".to_string()));
    assert!(!manifest.dependencies.runtime.is_empty());
    assert!(manifest
        .dependencies
        .runtime
        .iter()
        .any(|d| d.contains("libssl")));
    assert!(manifest
        .dependencies
        .runtime
        .iter()
        .any(|d| d.contains("zlib")));

    Ok(())
}

#[tokio::test]
async fn test_version_parsing_and_constraints() -> Result<(), Box<dyn std::error::Error>> {
    // Test version parsing
    eprintln!("Parsing version 2.1.3");
    let version = Version::parse("2.1.3")?;
    assert_eq!(version.major, 2);
    assert_eq!(version.minor, 1);
    assert_eq!(version.patch, 3);

    // Test package specs with version constraints
    eprintln!("Parsing package spec: pkg>=1.1.1,<2.0.0");
    let spec1 = PackageSpec::parse("pkg>=1.1.1,<2.0.0")?;
    assert_eq!(spec1.name, "pkg");
    eprintln!("Parsing versions for comparison");
    let v1 = Version::parse("1.1.1")?;
    let v2 = Version::parse("1.5.0")?;
    let v3 = Version::parse("2.0.0")?;

    assert!(spec1.version_spec.matches(&v1));
    assert!(spec1.version_spec.matches(&v2));
    assert!(!spec1.version_spec.matches(&v3));

    // Test compatible spec
    let spec2 = PackageSpec::parse("otherpkg~=1.2.0")?;
    assert_eq!(spec2.name, "otherpkg");
    let v4 = Version::parse("1.2.5")?;
    let v5 = Version::parse("1.3.0")?;

    assert!(spec2.version_spec.matches(&v4));
    assert!(!spec2.version_spec.matches(&v5));

    Ok(())
}

#[tokio::test]
async fn test_package_spec_parsing() -> Result<(), Box<dyn std::error::Error>> {
    // Test parsing package specifications
    let spec1 = PackageSpec::parse("curl>=8.0.0")?;
    assert_eq!(spec1.name, "curl");

    let spec2 = PackageSpec::parse("jq==1.7.0")?;
    assert_eq!(spec2.name, "jq");

    let spec3 = PackageSpec::parse("sqlite~=3.36.0")?;
    assert_eq!(spec3.name, "sqlite");

    // Test version parsing
    let version = Version::parse("2.1.3")?;
    assert_eq!(version.major, 2);
    assert_eq!(version.minor, 1);
    assert_eq!(version.patch, 3);

    Ok(())
}

#[tokio::test]
async fn test_content_hashing() -> Result<(), Box<dyn std::error::Error>> {
    // Test file hashing
    let temp_file = NamedTempFile::new()?;
    let data = b"Hello, World!";
    tokio::fs::write(temp_file.path(), data).await?;

    let hash1 = Hash::hash_file(temp_file.path()).await?;
    let hash2 = Hash::hash_file(temp_file.path()).await?;

    // Same file should produce same hash
    assert_eq!(hash1, hash2);

    // Test hash string format
    let hash_str = hash1.to_hex();
    assert_eq!(hash_str.len(), 64); // 32 bytes = 64 hex chars

    // Different data should produce different hash
    let temp_file2 = NamedTempFile::new()?;
    tokio::fs::write(temp_file2.path(), b"Goodbye, World!").await?;
    let hash3 = Hash::hash_file(temp_file2.path()).await?;
    assert_ne!(hash1, hash3);

    Ok(())
}

#[tokio::test]
async fn test_sbom_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnvironment::new().await?;

    // Load test SBOM data from fixtures
    let spdx_path = TestEnvironment::fixtures_path()
        .join("sboms")
        .join("hello-world-spdx.json");
    let spdx_content = fs::read_to_string(spdx_path).await?;

    // Basic SBOM parsing validation
    assert!(spdx_content.contains("spdxVersion"));
    assert!(spdx_content.contains("SPDX-2.3"));

    // Load CycloneDX SBOM
    let cdx_path = TestEnvironment::fixtures_path()
        .join("sboms")
        .join("complex-app-cyclonedx.json");
    let cdx_content = fs::read_to_string(cdx_path).await?;

    assert!(cdx_content.contains("bomFormat"));
    assert!(cdx_content.contains("CycloneDX"));

    Ok(())
}

#[tokio::test]
async fn test_index_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnvironment::new().await?;

    let index_data = env.load_test_index().await?;
    
    // Basic index validation
    assert!(index_data.contains("packages") || index_data.len() > 10);

    // Parse as JSON to verify structure
    let json_value: serde_json::Value = serde_json::from_str(&index_data)?;
    assert!(json_value.is_object() || json_value.is_array());

    Ok(())
}

#[tokio::test]
async fn test_vulnerability_data_loading() -> Result<(), Box<dyn std::error::Error>> {
    // Load test vulnerability data
    let vuln_path = TestEnvironment::fixtures_path()
        .join("vulnerabilities")
        .join("sample-vulns.json");
    let vuln_content = fs::read_to_string(vuln_path).await?;

    // Basic vulnerability data validation
    assert!(!vuln_content.is_empty());
    
    // Parse as JSON to verify structure
    let json_value: serde_json::Value = serde_json::from_str(&vuln_content)?;
    assert!(json_value.is_object() || json_value.is_array());

    Ok(())
}

#[tokio::test]
async fn test_large_manifest_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnvironment::new().await?;

    // Test that complex manifest with many dependencies parses correctly
    let complex_manifest = env.load_test_manifest("complex-app-2.1.3").await?;
    let manifest = sps2_manifest::Manifest::from_toml(&complex_manifest)?;

    // Should handle complex dependency structures
    assert_eq!(manifest.package.name, "complex-app");
    assert!(!manifest.dependencies.runtime.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_invalid_version_handling() {
    use sps2_types::Version;

    // Test invalid version strings
    assert!(Version::parse("").is_err());
    assert!(Version::parse("1").is_err());
    assert!(Version::parse("1.2").is_err());
    assert!(Version::parse("1.2.3.4").is_err());
    assert!(Version::parse("x.y.z").is_err());
    assert!(Version::parse("1.2.x").is_err());
}

#[tokio::test]
async fn test_invalid_manifest_handling() {
    // Test malformed TOML
    let invalid_toml = r#"
[package
name = "test"
"#;
    assert!(sps2_manifest::Manifest::from_toml(invalid_toml).is_err());

    // Test missing required fields
    let incomplete_manifest = r#"
[package]
# Missing name and version
description = "Test package"
"#;
    assert!(sps2_manifest::Manifest::from_toml(incomplete_manifest).is_err());
}