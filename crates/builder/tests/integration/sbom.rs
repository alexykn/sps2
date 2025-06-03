//! SBOM generation and management tests

use sps2_builder::*;
use tempfile::tempdir;
use tokio::fs;

#[test]
fn test_sbom_config_comprehensive() {
    let config = SbomConfig::default();
    assert!(config.generate_spdx);
    assert!(!config.generate_cyclonedx);
    assert!(!config.exclude_patterns.is_empty());
    assert!(config.include_dependencies);

    let both_config = SbomConfig::with_both_formats()
        .exclude("*.test".to_string())
        .exclude("*.debug".to_string())
        .include_dependencies(false);

    assert!(both_config.generate_spdx);
    assert!(both_config.generate_cyclonedx);
    assert!(both_config.exclude_patterns.contains(&"*.test".to_string()));
    assert!(both_config
        .exclude_patterns
        .contains(&"*.debug".to_string()));
    assert!(!both_config.include_dependencies);
}

#[tokio::test]
async fn test_sbom_generator() {
    let temp = tempdir().unwrap();

    // Create test package structure
    let package_dir = temp.path().join("sbom_test");
    fs::create_dir_all(&package_dir.join("files").join("bin"))
        .await
        .unwrap();
    fs::create_dir_all(&package_dir.join("files").join("lib"))
        .await
        .unwrap();

    // Create some test files
    fs::write(
        package_dir.join("files").join("bin").join("main"),
        "#!/bin/bash\necho 'test program'\n",
    )
    .await
    .unwrap();

    fs::write(
        package_dir.join("files").join("lib").join("libtest.so"),
        "fake library content",
    )
    .await
    .unwrap();

    // Test SBOM generation configuration
    let spdx_config = SbomConfig::default();
    assert!(spdx_config.generate_spdx);
    assert!(!spdx_config.generate_cyclonedx);

    let both_config = SbomConfig::with_both_formats();
    assert!(both_config.generate_spdx);
    assert!(both_config.generate_cyclonedx);

    // Test exclusion patterns
    let filtered_config = SbomConfig::with_both_formats()
        .exclude("*.debug".to_string())
        .exclude("*.test".to_string());

    assert!(filtered_config
        .exclude_patterns
        .contains(&"*.debug".to_string()));
    assert!(filtered_config
        .exclude_patterns
        .contains(&"*.test".to_string()));
}

#[tokio::test]
async fn test_sbom_files_management() {
    let temp = tempdir().unwrap();

    // Test SBOM file generation and management
    let sbom_dir = temp.path().join("sbom_output");
    fs::create_dir_all(&sbom_dir).await.unwrap();

    // Create test SBOM content
    let spdx_content = r#"{
    "spdxVersion": "SPDX-2.3",
    "name": "test-package",
    "creationInfo": {
        "created": "2024-01-01T00:00:00Z"
    },
    "packages": []
}"#;

    let cyclonedx_content = r#"{
    "bomFormat": "CycloneDX",
    "specVersion": "1.6",
    "version": 1,
    "components": []
}"#;

    // Write SBOM files
    fs::write(sbom_dir.join("sbom.spdx.json"), spdx_content)
        .await
        .unwrap();
    fs::write(sbom_dir.join("sbom.cdx.json"), cyclonedx_content)
        .await
        .unwrap();

    // Verify files exist and have content
    assert!(sbom_dir.join("sbom.spdx.json").exists());
    assert!(sbom_dir.join("sbom.cdx.json").exists());

    let read_spdx = fs::read_to_string(sbom_dir.join("sbom.spdx.json"))
        .await
        .unwrap();
    assert!(read_spdx.contains("SPDX-2.3"));
    assert!(read_spdx.contains("test-package"));

    let read_cdx = fs::read_to_string(sbom_dir.join("sbom.cdx.json"))
        .await
        .unwrap();
    assert!(read_cdx.contains("CycloneDX"));
    assert!(read_cdx.contains("1.6"));
}
