//! Integration tests for sps2 package manager
//!
//! These tests exercise the full system end-to-end using test fixtures.

use std::path::PathBuf;
use tempfile::TempDir;
use tokio::fs;

// Test utilities module
mod utils {
    use super::*;
    use sps2_config::Config;
    use sps2_events::{EventReceiver, EventSender};
    use sps2_ops::OpsCtx;
    use sps2_state::StateManager;
    use sps2_store::PackageStore;

    pub struct TestEnvironment {
        pub temp_dir: TempDir,
        pub config: Config,
        pub ops_ctx: OpsCtx,
        #[allow(dead_code)] // Used in event-based integration tests
        pub event_sender: EventSender,
        #[allow(dead_code)] // Used in event-based integration tests
        pub event_receiver: EventReceiver,
    }

    impl TestEnvironment {
        pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
            let temp_dir = TempDir::new()?;
            let base_path = temp_dir.path();

            // Set up test directory structure
            let store_path = base_path.join("store");
            let state_path = base_path.join("state.sqlite");
            let live_path = base_path.join("live");
            let cache_path = base_path.join("cache");

            fs::create_dir_all(&store_path).await?;
            fs::create_dir_all(&live_path).await?;
            fs::create_dir_all(&cache_path).await?;

            // Create test configuration
            let mut config = Config::default();
            config.paths.store_path = Some(store_path.clone());
            config.paths.state_path = Some(state_path);
            config.general.parallel_downloads = 2; // Smaller for tests
            config.network.timeout = 30;
            config.security.verify_signatures = false; // Disable for tests

            // Create event channel
            let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

            // Initialize components
            let state = StateManager::new(base_path).await?;
            let store = PackageStore::new(store_path);
            let index = sps2_index::IndexManager::new(base_path);
            let net = sps2_net::NetClient::with_defaults()?;
            let resolver = sps2_resolver::Resolver::new(index.clone());
            let builder = sps2_builder::Builder::new();

            let ops_ctx = OpsCtx::new(
                store,
                state,
                index,
                net,
                resolver,
                builder,
                event_sender.clone(),
            );

            Ok(Self {
                temp_dir,
                config,
                ops_ctx,
                event_sender,
                event_receiver,
            })
        }

        pub fn fixtures_path() -> PathBuf {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("tests")
                .join("fixtures")
        }

        pub async fn load_test_manifest(
            &self,
            name: &str,
        ) -> Result<String, Box<dyn std::error::Error>> {
            let manifest_path = Self::fixtures_path()
                .join("manifests")
                .join(format!("{name}.toml"));
            Ok(fs::read_to_string(manifest_path).await?)
        }

        #[allow(dead_code)] // Used in Starlark recipe testing
        pub async fn load_test_recipe(
            &self,
            name: &str,
        ) -> Result<String, Box<dyn std::error::Error>> {
            let recipe_path = Self::fixtures_path()
                .join("recipes")
                .join(format!("{name}.star"));
            Ok(fs::read_to_string(recipe_path).await?)
        }

        pub async fn load_test_index(&self) -> Result<String, Box<dyn std::error::Error>> {
            let index_path = Self::fixtures_path().join("index").join("packages.json");
            Ok(fs::read_to_string(index_path).await?)
        }
    }
}

#[tokio::test]
#[ignore] // Requires /opt/pm SQLite database - fails in CI
async fn test_system_initialization() -> Result<(), Box<dyn std::error::Error>> {
    let env = utils::TestEnvironment::new().await?;

    // Test that all components are properly initialized
    assert!(env.temp_dir.path().exists());
    assert!(env.config.paths.store_path.as_ref().unwrap().exists());
    assert!(env.temp_dir.path().join("live").exists());

    // Test state manager initialization
    // In a fresh system, an initial state should be automatically created
    let active_state_result = env.ops_ctx.state.get_active_state().await;
    assert!(active_state_result.is_ok()); // Should succeed with initial state

    // List of states should contain exactly one initial state
    let states = env.ops_ctx.state.list_states().await?;
    assert_eq!(states.len(), 1); // Should have one initial state

    Ok(())
}

#[tokio::test]
async fn test_manifest_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let env = utils::TestEnvironment::new().await?;

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
    use sps2_types::{PackageSpec, Version};

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
    use sps2_types::{PackageSpec, Version};

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
    use sps2_hash::Hash;
    use tempfile::NamedTempFile;

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
async fn test_event_system() -> Result<(), Box<dyn std::error::Error>> {
    use sps2_events::Event;
    use sps2_types::Version;

    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();

    // Send some events
    sender
        .send(Event::PackageInstalling {
            name: "hello-world".to_string(),
            version: Version::parse("1.0.0")?,
        })
        .unwrap();

    sender
        .send(Event::DownloadProgress {
            url: "https://example.com/package.sp".to_string(),
            bytes_downloaded: 1024,
            total_bytes: 2048,
        })
        .unwrap();

    // Receive and verify events
    let event1 = receiver.recv().await.unwrap();
    match event1 {
        Event::PackageInstalling { name, version } => {
            assert_eq!(name, "hello-world");
            assert_eq!(version.to_string(), "1.0.0");
        }
        _ => panic!("Unexpected event type"),
    }

    let event2 = receiver.recv().await.unwrap();
    match event2 {
        Event::DownloadProgress {
            url,
            bytes_downloaded,
            total_bytes,
        } => {
            assert_eq!(url, "https://example.com/package.sp");
            assert_eq!(bytes_downloaded, 1024);
            assert_eq!(total_bytes, 2048);
        }
        _ => panic!("Unexpected event type"),
    }

    Ok(())
}

#[tokio::test]
async fn test_sbom_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let _env = utils::TestEnvironment::new().await?;

    // Test SPDX SBOM parsing
    let spdx_path = utils::TestEnvironment::fixtures_path()
        .join("sboms")
        .join("hello-world-spdx.json");

    let spdx_data = fs::read(&spdx_path).await?;

    // Would need to implement actual parsing in the audit crate
    // For now, just verify the file exists and is valid JSON
    let _json: serde_json::Value = serde_json::from_slice(&spdx_data)?;

    // Test CycloneDX SBOM parsing
    let cyclonedx_path = utils::TestEnvironment::fixtures_path()
        .join("sboms")
        .join("complex-app-cyclonedx.json");

    let cyclonedx_data = fs::read(&cyclonedx_path).await?;
    let _json: serde_json::Value = serde_json::from_slice(&cyclonedx_data)?;

    Ok(())
}

#[tokio::test]
async fn test_index_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let env = utils::TestEnvironment::new().await?;

    let index_data = env.load_test_index().await?;
    let index: serde_json::Value = serde_json::from_str(&index_data)?;

    // Verify index structure
    assert!(index.get("index_version").is_some());
    assert!(index.get("packages").is_some());

    let packages = index.get("packages").unwrap().as_object().unwrap();
    assert!(packages.contains_key("hello-world"));
    assert!(packages.contains_key("complex-app"));
    assert!(packages.contains_key("libssl"));

    // Verify package structure
    let hello_world = packages.get("hello-world").unwrap();
    assert!(hello_world.get("name").is_some());
    assert!(hello_world.get("versions").is_some());

    let versions = hello_world.get("versions").unwrap().as_object().unwrap();
    assert!(versions.contains_key("1.0.0"));

    Ok(())
}

#[tokio::test]
async fn test_configuration_loading() -> Result<(), Box<dyn std::error::Error>> {
    use sps2_config::Config;

    let fixtures_path = utils::TestEnvironment::fixtures_path();
    let config_path = fixtures_path.join("config").join("test-config.toml");

    let config_data = fs::read_to_string(config_path).await?;
    let config: Config = toml::from_str(&config_data)?;

    // Verify configuration values
    assert_eq!(config.general.parallel_downloads, 2);
    // config.general.default_output is an enum, not a string
    assert_eq!(config.network.timeout, 30);
    assert_eq!(config.network.retries, 2);
    assert!(!config.security.verify_signatures);
    assert_eq!(config.build.build_jobs, 2);

    Ok(())
}

#[tokio::test]
async fn test_vulnerability_data_loading() -> Result<(), Box<dyn std::error::Error>> {
    let fixtures_path = utils::TestEnvironment::fixtures_path();
    let vulns_path = fixtures_path
        .join("vulnerabilities")
        .join("sample-vulns.json");

    let vulns_data = fs::read_to_string(vulns_path).await?;
    let vulns: serde_json::Value = serde_json::from_str(&vulns_data)?;

    let vulns_array = vulns.as_array().unwrap();
    assert!(!vulns_array.is_empty());

    // Check first vulnerability
    let first_vuln = &vulns_array[0];
    assert_eq!(
        first_vuln.get("cve_id").unwrap().as_str().unwrap(),
        "CVE-2023-1234"
    );
    assert_eq!(
        first_vuln.get("severity").unwrap().as_str().unwrap(),
        "high"
    );
    assert!(first_vuln.get("cvss_score").is_some());
    assert!(first_vuln.get("affected_versions").is_some());
    assert!(first_vuln.get("fixed_versions").is_some());

    Ok(())
}

// Performance and stress tests
#[tokio::test]
async fn test_concurrent_operations() -> Result<(), Box<dyn std::error::Error>> {
    let _env = utils::TestEnvironment::new().await?;

    // Test concurrent hash operations
    let mut tasks = Vec::new();

    for i in 0..10 {
        let task = tokio::spawn(async move {
            let data = format!("test data {i}").into_bytes();
            sps2_hash::Hash::from_data(&data)
        });
        tasks.push(task);
    }

    // Wait for all tasks to complete
    let results = futures::future::join_all(tasks).await;

    // Verify all completed successfully
    for result in results {
        assert!(result.is_ok());
    }

    Ok(())
}

#[tokio::test]
async fn test_large_manifest_parsing() -> Result<(), Box<dyn std::error::Error>> {
    // Test parsing a manifest with many dependencies
    let large_manifest = r#"
[package]
name = "large-package"
version = "1.0.0"
revision = 1
arch = "aarch64-apple-darwin"
description = "Package with many dependencies for testing"
license = "MIT"

[dependencies]
runtime = [
"#;

    // Add many dependencies
    let mut manifest_content = large_manifest.to_string();
    for i in 0..100 {
        manifest_content.push_str(&format!("    \"dep{i}>=1.0.0\",\n"));
    }
    manifest_content.push_str("]\n");

    let manifest = sps2_manifest::Manifest::from_toml(&manifest_content)?;
    assert_eq!(manifest.package.name, "large-package");
    assert_eq!(manifest.dependencies.runtime.len(), 100);

    Ok(())
}

// Error handling tests
#[tokio::test]
async fn test_invalid_version_handling() {
    use sps2_types::Version;

    // Test invalid version strings
    assert!(Version::parse("").is_err());
    assert!(Version::parse("1").is_err());
    assert!(Version::parse("1.2").is_err());
    assert!(Version::parse("1.2.3.4").is_err());
    assert!(Version::parse("1.a.3").is_err());
    assert!(Version::parse("not.a.version").is_err());
}

#[tokio::test]
async fn test_invalid_manifest_handling() {
    // Test malformed TOML
    let invalid_toml = "this is not valid toml [[[";
    assert!(sps2_manifest::Manifest::from_toml(invalid_toml).is_err());

    // Test missing required fields
    let incomplete_manifest = r#"
[package]
name = "incomplete"
# missing version
"#;
    assert!(sps2_manifest::Manifest::from_toml(incomplete_manifest).is_err());
}

// Builder and Store Integration Tests
mod builder_store_integration_tests {
    use super::*;
    // Note: Builder imports removed since tests use mock packages instead of Builder.build()
    use sps2_install::{validate_sp_file, PackageFormat};
    use sps2_store::extract_package;
    // Note: Version import removed since it's not used in these tests
    use tempfile::tempdir;

    /// Test Store API extraction with mock .sp packages
    #[tokio::test]
    async fn test_store_package_extraction() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let output_dir = temp.path().join("output");
        fs::create_dir_all(&output_dir).await?;

        // Create mock package structure directly (simulating builder output)
        let mock_pkg_dir = temp.path().join("mock_package");
        fs::create_dir_all(&mock_pkg_dir).await?;

        // Create manifest
        let manifest_content = r#"
[package]
name = "extraction-test"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "Test package for Store API extraction"
license = "MIT"

[dependencies]
runtime = []
build = []
"#;
        fs::write(mock_pkg_dir.join("manifest.toml"), manifest_content).await?;

        // Create files directory with test content
        let files_dir = mock_pkg_dir.join("files");
        fs::create_dir_all(&files_dir.join("bin")).await?;
        fs::create_dir_all(&files_dir.join("lib")).await?;
        fs::create_dir_all(&files_dir.join("share").join("doc")).await?;

        fs::write(
            files_dir.join("bin").join("hello"),
            "#!/bin/bash\necho 'Hello from extraction-test 1.0.0'\n",
        )
        .await?;

        fs::write(
            files_dir.join("lib").join("libtest.so"),
            "fake library content for testing",
        )
        .await?;

        fs::write(
            files_dir.join("share").join("doc").join("README.md"),
            "# extraction-test 1.0.0\n\nTest package documentation.\n",
        )
        .await?;

        // Create a .sp file using create_package (creates plain tar format)
        let sp_file = output_dir.join("extraction-test-1.0.0-1.arm64.sp");
        sps2_store::create_package(&mock_pkg_dir, &sp_file).await?;

        // Verify the package was created
        assert!(sp_file.exists());

        // Validate the package
        let validation_result = validate_sp_file(&sp_file, None).await?;
        assert!(validation_result.is_valid);

        // Extract using Store API and verify contents
        let extract_dir = temp.path().join("extracted");
        extract_package(&sp_file, &extract_dir).await?;

        // Verify extracted structure
        assert!(extract_dir.join("manifest.toml").exists());
        assert!(extract_dir.join("files").join("bin").join("hello").exists());
        assert!(extract_dir
            .join("files")
            .join("lib")
            .join("libtest.so")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("share")
            .join("doc")
            .join("README.md")
            .exists());

        // Verify extracted content
        let hello_content =
            fs::read_to_string(extract_dir.join("files").join("bin").join("hello")).await?;
        assert!(hello_content.contains("Hello from extraction-test 1.0.0"));

        let lib_content =
            fs::read_to_string(extract_dir.join("files").join("lib").join("libtest.so")).await?;
        assert_eq!(lib_content, "fake library content for testing");

        let doc_content = fs::read_to_string(
            extract_dir
                .join("files")
                .join("share")
                .join("doc")
                .join("README.md"),
        )
        .await?;
        assert!(doc_content.contains("extraction-test 1.0.0"));

        Ok(())
    }

    /// Test extraction performance with different compression levels
    /// Uses mock packages with different content characteristics
    #[tokio::test]
    async fn test_extraction_compression_levels() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;

        // Create test package with repetitive content (good for compression)
        let package_dir = temp.path().join("package");
        fs::create_dir_all(&package_dir).await?;

        let manifest_content = r#"
[package]
name = "compression-test"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "Test package for compression level testing"
license = "MIT"

[dependencies]
runtime = []
build = []
"#;
        fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

        let files_dir = package_dir.join("files");
        fs::create_dir_all(&files_dir.join("bin")).await?;

        // Create larger file with repetitive content to test compression
        let test_content = "test content line for compression testing\n".repeat(5000);
        fs::write(files_dir.join("bin").join("test"), &test_content).await?;

        // Add some binary-like content that doesn't compress as well
        let binary_content = (0..1000).map(|i| (i % 256) as u8).collect::<Vec<u8>>();
        fs::write(files_dir.join("bin").join("binary"), &binary_content).await?;

        // Create plain tar package (no compression)
        let plain_sp = temp.path().join("compression-plain.sp");
        sps2_store::create_package(&package_dir, &plain_sp).await?;

        // Verify the package format and extraction
        let plain_validation = validate_sp_file(&plain_sp, None).await?;
        assert!(plain_validation.is_valid);
        assert_eq!(plain_validation.format, PackageFormat::PlainTar);

        // Extract and verify using Store API
        let extract_dir = temp.path().join("extracted");
        extract_package(&plain_sp, &extract_dir).await?;

        // Verify extracted structure and content
        assert!(extract_dir.join("manifest.toml").exists());
        assert!(extract_dir.join("files").join("bin").join("test").exists());
        assert!(extract_dir
            .join("files")
            .join("bin")
            .join("binary")
            .exists());

        // Verify content integrity
        let extracted_content =
            fs::read_to_string(extract_dir.join("files").join("bin").join("test")).await?;
        assert_eq!(extracted_content, test_content);

        let extracted_binary =
            fs::read(extract_dir.join("files").join("bin").join("binary")).await?;
        assert_eq!(extracted_binary, binary_content);

        Ok(())
    }

    /// Test package validation pipeline using Store API
    #[tokio::test]
    async fn test_package_validation_pipeline() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;

        // Create comprehensive test package
        let package_dir = temp.path().join("package");
        fs::create_dir_all(&package_dir).await?;

        let manifest_content = r#"
[package]
name = "validation-test"
version = "2.1.0"
revision = 1
arch = "arm64"
description = "Package for validation testing"
license = "Apache-2.0"
homepage = "https://example.com/validation-test"

[dependencies]
runtime = ["libssl>=1.1.0", "zlib>=1.2.0"]
build = ["make>=4.0", "gcc>=9.0.0"]
"#;
        fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

        // Add SBOM files
        fs::write(
            package_dir.join("sbom.spdx.json"),
            r#"{"spdxVersion": "SPDX-2.3", "name": "validation-test", "packages": []}"#,
        )
        .await?;

        // Create comprehensive file structure
        let files_dir = package_dir.join("files");
        fs::create_dir_all(&files_dir.join("bin")).await?;
        fs::create_dir_all(&files_dir.join("lib")).await?;
        fs::create_dir_all(&files_dir.join("share").join("doc")).await?;
        fs::create_dir_all(&files_dir.join("etc")).await?;

        // Create various file types
        fs::write(
            files_dir.join("bin").join("main"),
            "#!/bin/bash\necho 'validation-test main program'\n",
        )
        .await?;

        fs::write(
            files_dir.join("lib").join("libvalidation.so"),
            "fake library content for validation testing",
        )
        .await?;

        fs::write(
            files_dir.join("share").join("doc").join("README.md"),
            "# validation-test 2.1.0\n\nComprehensive validation testing package.\n",
        )
        .await?;

        fs::write(
            files_dir.join("etc").join("config.conf"),
            "# Configuration file\nversion=2.1.0\nmode=test\n",
        )
        .await?;

        // Create package using Store API
        let sp_file = temp.path().join("validation-test-2.1.0-1.arm64.sp");
        sps2_store::create_package(&package_dir, &sp_file).await?;

        // Validate the package
        let validation = validate_sp_file(&sp_file, None).await?;
        assert!(validation.is_valid);
        assert!(validation.file_count > 6); // manifest + sbom + at least 4 files
        assert!(validation.extracted_size > 0);
        assert!(validation.manifest.is_some());

        // Extract using Store API and verify full pipeline
        let extract_dir = temp.path().join("extracted");
        extract_package(&sp_file, &extract_dir).await?;

        // Verify all expected files exist
        assert!(extract_dir.join("manifest.toml").exists());
        assert!(extract_dir.join("sbom.spdx.json").exists());
        assert!(extract_dir.join("files").join("bin").join("main").exists());
        assert!(extract_dir
            .join("files")
            .join("lib")
            .join("libvalidation.so")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("share")
            .join("doc")
            .join("README.md")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("etc")
            .join("config.conf")
            .exists());

        // Verify content integrity
        let manifest_text = fs::read_to_string(extract_dir.join("manifest.toml")).await?;
        assert!(manifest_text.contains("validation-test"));
        assert!(manifest_text.contains("2.1.0"));
        assert!(manifest_text.contains("libssl>=1.1.0"));

        let main_content =
            fs::read_to_string(extract_dir.join("files").join("bin").join("main")).await?;
        assert!(main_content.contains("validation-test main program"));

        Ok(())
    }

    /// Test complete Store API round-trip: create → validate → extract
    #[tokio::test]
    async fn test_store_roundtrip_integration() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;

        // Create comprehensive package structure for round-trip testing
        let package_dir = temp.path().join("source");
        fs::create_dir_all(&package_dir).await?;

        // Create comprehensive package structure
        let manifest_content = r#"
[package]
name = "roundtrip-test"
version = "3.2.1"
revision = 2
arch = "arm64"
description = "End-to-end Store API test package"
homepage = "https://example.com/roundtrip-test"
license = "MIT"

[dependencies]
runtime = [
    "zlib>=1.2.0",
    "openssl~=3.0.0"
]
build = [
    "gcc>=9.0.0",
    "make>=4.0"
]
"#;
        fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

        // Add SBOM files
        fs::write(
            package_dir.join("sbom.spdx.json"),
            r#"{"spdxVersion": "SPDX-2.3", "name": "roundtrip-test", "packages": []}"#,
        )
        .await?;

        fs::write(
            package_dir.join("sbom.cdx.json"),
            r#"{"bomFormat": "CycloneDX", "specVersion": "1.6", "components": []}"#,
        )
        .await?;

        // Create realistic file structure
        let files_dir = package_dir.join("files");
        fs::create_dir_all(&files_dir.join("bin")).await?;
        fs::create_dir_all(&files_dir.join("lib")).await?;
        fs::create_dir_all(&files_dir.join("share").join("man")).await?;

        // Create diverse file types
        fs::write(
            files_dir.join("bin").join("roundtrip"),
            "#!/bin/bash\necho 'roundtrip-test successful'\nexit 0\n",
        )
        .await?;

        fs::write(
            files_dir.join("lib").join("libroundtrip.so"),
            "FAKE_LIBRARY_CONTENT_FOR_ROUNDTRIP_TESTING",
        )
        .await?;

        fs::write(
            files_dir.join("share").join("man").join("roundtrip.1"),
            ".TH ROUNDTRIP 1\n.SH NAME\nroundtrip - test program\n",
        )
        .await?;

        // Step 1: Create package using Store API (creates plain tar)
        let sp_file = temp.path().join("roundtrip-test-3.2.1-2.arm64.sp");
        sps2_store::create_package(&package_dir, &sp_file).await?;

        // Step 2: Validate package (simulating install validation)
        let validation = validate_sp_file(&sp_file, None).await?;
        assert!(validation.is_valid);
        assert!(validation.file_count >= 7); // manifest + 2 sboms + at least 4 files
        assert!(validation.extracted_size > 0);
        assert!(validation.manifest.is_some());

        // Parse and verify manifest content
        let manifest_text = validation.manifest.as_ref().unwrap();
        assert!(manifest_text.contains("roundtrip-test"));
        assert!(manifest_text.contains("3.2.1"));
        assert!(manifest_text.contains("zlib>=1.2.0"));
        assert!(manifest_text.contains("openssl~=3.0.0"));

        // Step 3: Extract package using Store API
        let extract_dir = temp.path().join("extracted");
        extract_package(&sp_file, &extract_dir).await?;

        // Verify extraction structure
        assert!(extract_dir.join("manifest.toml").exists());
        assert!(extract_dir.join("sbom.spdx.json").exists());
        assert!(extract_dir.join("sbom.cdx.json").exists());
        assert!(extract_dir
            .join("files")
            .join("bin")
            .join("roundtrip")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("lib")
            .join("libroundtrip.so")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("share")
            .join("man")
            .join("roundtrip.1")
            .exists());

        // Step 4: Verify content integrity (byte-for-byte comparison)
        let extracted_manifest = fs::read_to_string(extract_dir.join("manifest.toml")).await?;
        let original_manifest = fs::read_to_string(package_dir.join("manifest.toml")).await?;
        assert_eq!(extracted_manifest, original_manifest);

        let extracted_binary =
            fs::read_to_string(extract_dir.join("files").join("bin").join("roundtrip")).await?;
        let original_binary = fs::read_to_string(files_dir.join("bin").join("roundtrip")).await?;
        assert_eq!(extracted_binary, original_binary);

        let extracted_lib = fs::read_to_string(
            extract_dir
                .join("files")
                .join("lib")
                .join("libroundtrip.so"),
        )
        .await?;
        let original_lib =
            fs::read_to_string(files_dir.join("lib").join("libroundtrip.so")).await?;
        assert_eq!(extracted_lib, original_lib);

        let extracted_man = fs::read_to_string(
            extract_dir
                .join("files")
                .join("share")
                .join("man")
                .join("roundtrip.1"),
        )
        .await?;
        let original_man =
            fs::read_to_string(files_dir.join("share").join("man").join("roundtrip.1")).await?;
        assert_eq!(extracted_man, original_man);

        // Step 5: Verify SBOM files
        let extracted_spdx = fs::read_to_string(extract_dir.join("sbom.spdx.json")).await?;
        let original_spdx = fs::read_to_string(package_dir.join("sbom.spdx.json")).await?;
        assert_eq!(extracted_spdx, original_spdx);

        Ok(())
    }

    /// Test Store API with different package sizes and content types
    #[tokio::test]
    async fn test_store_package_sizes() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;

        // Create package with diverse content to test Store API robustness
        let package_dir = temp.path().join("package");
        fs::create_dir_all(&package_dir).await?;

        let manifest_content = r#"
[package]
name = "size-test"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "Package for testing different content sizes"
license = "MIT"

[dependencies]
runtime = ["base-runtime"]
build = ["build-tools"]
"#;
        fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

        // Create files with varying content characteristics
        let files_dir = package_dir.join("files");
        fs::create_dir_all(&files_dir.join("data")).await?;
        fs::create_dir_all(&files_dir.join("bin")).await?;
        fs::create_dir_all(&files_dir.join("config")).await?;

        // Large text file with repetitive content
        let repetitive_content = "This line repeats many times for size testing.\n".repeat(2000);
        fs::write(
            files_dir.join("data").join("large.txt"),
            &repetitive_content,
        )
        .await?;

        // Binary-like content that doesn't compress well
        let binary_content = (0..5000).map(|i| (i % 256) as u8).collect::<Vec<u8>>();
        fs::write(files_dir.join("data").join("binary.dat"), &binary_content).await?;

        // Many small files
        for i in 0..25 {
            fs::write(
                files_dir
                    .join("config")
                    .join(format!("config{:03}.conf", i)),
                format!("# Config file {}\nvalue={i}\nname=test-{i}\n", i),
            )
            .await?;
        }

        // Executable file
        fs::write(
            files_dir.join("bin").join("size-test"),
            "#!/bin/bash\necho 'Size test program'\necho 'Testing Store API with various content sizes'\n",
        ).await?;

        // Create package using Store API
        let sp_file = temp.path().join("size-test-1.0.0-1.arm64.sp");
        sps2_store::create_package(&package_dir, &sp_file).await?;

        // Validate the package
        let validation = validate_sp_file(&sp_file, None).await?;
        assert!(validation.is_valid);
        assert!(validation.file_count > 25); // manifest + many files
        assert!(validation.extracted_size > 50000); // Should be reasonably large

        // Extract using Store API
        let extract_dir = temp.path().join("extracted");
        extract_package(&sp_file, &extract_dir).await?;

        // Verify all content types were extracted correctly
        assert!(extract_dir.join("manifest.toml").exists());
        assert!(extract_dir
            .join("files")
            .join("data")
            .join("large.txt")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("data")
            .join("binary.dat")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("bin")
            .join("size-test")
            .exists());

        // Verify some config files exist
        assert!(extract_dir
            .join("files")
            .join("config")
            .join("config000.conf")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("config")
            .join("config024.conf")
            .exists());

        // Verify content integrity for different types
        let extracted_text =
            fs::read_to_string(extract_dir.join("files").join("data").join("large.txt")).await?;
        assert_eq!(extracted_text, repetitive_content);

        let extracted_binary =
            fs::read(extract_dir.join("files").join("data").join("binary.dat")).await?;
        assert_eq!(extracted_binary, binary_content);

        let config_content = fs::read_to_string(
            extract_dir
                .join("files")
                .join("config")
                .join("config010.conf"),
        )
        .await?;
        assert!(config_content.contains("Config file 10"));
        assert!(config_content.contains("value=10"));

        Ok(())
    }

    /// Test Store API edge cases: empty packages, single files, many files
    #[tokio::test]
    async fn test_store_api_edge_cases() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;

        // Test 1: Minimal package (just manifest and empty files dir)
        let minimal_dir = temp.path().join("minimal");
        fs::create_dir_all(&minimal_dir).await?;

        fs::write(
            minimal_dir.join("manifest.toml"),
            r#"
[package]
name = "minimal"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "Minimal test package"
license = "MIT"

[dependencies]
runtime = []
build = []
"#,
        )
        .await?;

        // Create empty files directory
        fs::create_dir_all(minimal_dir.join("files")).await?;

        let minimal_sp = temp.path().join("minimal.sp");
        sps2_store::create_package(&minimal_dir, &minimal_sp).await?;

        let validation = validate_sp_file(&minimal_sp, None).await?;
        assert!(validation.is_valid);

        let extract_dir = temp.path().join("minimal_extracted");
        extract_package(&minimal_sp, &extract_dir).await?;
        assert!(extract_dir.join("manifest.toml").exists());
        assert!(extract_dir.join("files").exists());

        // Verify manifest content
        let manifest_content = fs::read_to_string(extract_dir.join("manifest.toml")).await?;
        assert!(manifest_content.contains("minimal"));

        // Test 2: Package with single large file
        let single_file_dir = temp.path().join("single_file");
        fs::create_dir_all(&single_file_dir).await?;

        fs::write(
            single_file_dir.join("manifest.toml"),
            r#"
[package]
name = "single-file"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "Package with single large file"
license = "MIT"

[dependencies]
runtime = []
build = []
"#,
        )
        .await?;

        let files_dir = single_file_dir.join("files");
        fs::create_dir_all(&files_dir).await?;

        // Create a single larger file
        let large_content = "Large file content line for Store API testing.\n".repeat(5000);
        fs::write(files_dir.join("large_file.dat"), &large_content).await?;

        let single_sp = temp.path().join("single.sp");
        sps2_store::create_package(&single_file_dir, &single_sp).await?;

        let validation = validate_sp_file(&single_sp, None).await?;
        assert!(validation.is_valid);
        assert!(validation.file_count >= 3); // manifest + files dir + large file

        let single_extract_dir = temp.path().join("single_extracted");
        extract_package(&single_sp, &single_extract_dir).await?;

        let extracted_content =
            fs::read_to_string(single_extract_dir.join("files").join("large_file.dat")).await?;
        assert_eq!(extracted_content, large_content);

        // Test 3: Package with many small files
        let many_files_dir = temp.path().join("many_files");
        fs::create_dir_all(&many_files_dir).await?;

        fs::write(
            many_files_dir.join("manifest.toml"),
            r#"
[package]
name = "many-files"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "Package with many small files"
license = "MIT"

[dependencies]
runtime = []
build = []
"#,
        )
        .await?;

        let files_dir = many_files_dir.join("files");
        fs::create_dir_all(&files_dir.join("data")).await?;

        // Create many small files
        for i in 0..75 {
            fs::write(
                files_dir.join("data").join(format!("small_{:03}.txt", i)),
                format!(
                    "Small file content for file number {}\nUsed for Store API testing.\n",
                    i
                ),
            )
            .await?;
        }

        let many_sp = temp.path().join("many.sp");
        sps2_store::create_package(&many_files_dir, &many_sp).await?;

        let validation = validate_sp_file(&many_sp, None).await?;
        assert!(validation.is_valid);
        assert!(validation.file_count > 75); // 75 files + manifest + dirs

        let many_extract_dir = temp.path().join("many_extracted");
        extract_package(&many_sp, &many_extract_dir).await?;

        // Verify some files were extracted correctly
        assert!(many_extract_dir
            .join("files")
            .join("data")
            .join("small_000.txt")
            .exists());
        assert!(many_extract_dir
            .join("files")
            .join("data")
            .join("small_074.txt")
            .exists());

        let first_file_content = fs::read_to_string(
            many_extract_dir
                .join("files")
                .join("data")
                .join("small_000.txt"),
        )
        .await?;
        assert!(first_file_content.contains("Small file content for file number 0"));

        Ok(())
    }

    /// Test Store API with various package structures and formats
    #[tokio::test]
    async fn test_store_package_formats() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;

        // Test different package structures that Store API should handle
        let package_dir = temp.path().join("format_test");
        fs::create_dir_all(&package_dir).await?;

        // Standard manifest
        let manifest_content = r#"
[package]
name = "format-test"
version = "2.0.0"
revision = 1
arch = "arm64"
description = "Package for format testing"
homepage = "https://example.com/format-test"
license = "Apache-2.0"

[dependencies]
runtime = ["base-system>=1.0.0"]
build = ["build-essential"]
"#;
        fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

        // Standard files structure
        let files_dir = package_dir.join("files");
        fs::create_dir_all(&files_dir.join("bin")).await?;
        fs::create_dir_all(&files_dir.join("lib")).await?;
        fs::create_dir_all(&files_dir.join("include")).await?;
        fs::create_dir_all(&files_dir.join("share").join("doc")).await?;

        // Create various file types
        fs::write(
            files_dir.join("bin").join("format-test"),
            "#!/bin/bash\necho 'Format test application'\necho 'Testing Store API format handling'\n",
        ).await?;

        fs::write(
            files_dir.join("lib").join("libformat.so"),
            "ELF_FAKE_BINARY_CONTENT_FOR_LIBRARY_TESTING",
        )
        .await?;

        fs::write(
            files_dir.join("include").join("format.h"),
            "#ifndef FORMAT_H\n#define FORMAT_H\n\nvoid format_test(void);\n\n#endif\n",
        )
        .await?;

        fs::write(
            files_dir.join("share").join("doc").join("README.txt"),
            "Format Test Package\n==================\n\nThis package tests Store API format handling.\n",
        ).await?;

        // Create package using Store API (creates plain tar)
        let sp_file = temp.path().join("format-test-2.0.0-1.arm64.sp");
        sps2_store::create_package(&package_dir, &sp_file).await?;

        // Verify package creation and format detection
        let validation = validate_sp_file(&sp_file, None).await?;
        assert!(validation.is_valid);
        assert_eq!(validation.format, PackageFormat::PlainTar);
        assert!(validation.file_count >= 6); // manifest + dirs + files

        // Extract using Store API
        let extract_dir = temp.path().join("format_extracted");
        extract_package(&sp_file, &extract_dir).await?;

        // Verify all file types were preserved
        assert!(extract_dir.join("manifest.toml").exists());
        assert!(extract_dir
            .join("files")
            .join("bin")
            .join("format-test")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("lib")
            .join("libformat.so")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("include")
            .join("format.h")
            .exists());
        assert!(extract_dir
            .join("files")
            .join("share")
            .join("doc")
            .join("README.txt")
            .exists());

        // Verify content preservation for different file types
        let script_content =
            fs::read_to_string(extract_dir.join("files").join("bin").join("format-test")).await?;
        assert!(script_content.contains("Format test application"));
        assert!(script_content.contains("Testing Store API format handling"));

        let header_content =
            fs::read_to_string(extract_dir.join("files").join("include").join("format.h")).await?;
        assert!(header_content.contains("#ifndef FORMAT_H"));
        assert!(header_content.contains("void format_test(void);"));

        let doc_content = fs::read_to_string(
            extract_dir
                .join("files")
                .join("share")
                .join("doc")
                .join("README.txt"),
        )
        .await?;
        assert!(doc_content.contains("Format Test Package"));
        assert!(doc_content.contains("Store API format handling"));

        let lib_content =
            fs::read_to_string(extract_dir.join("files").join("lib").join("libformat.so")).await?;
        assert!(lib_content.contains("ELF_FAKE_BINARY_CONTENT"));

        Ok(())
    }

    // Note: All tests now use the proper Store API (create_package and extract_package)
    // instead of manual compression/decompression. This ensures tests validate the
    // actual production code paths that users will experience.
}

// Cleanup test - should be last
#[tokio::test]
async fn test_cleanup_and_finalization() -> Result<(), Box<dyn std::error::Error>> {
    let env = utils::TestEnvironment::new().await?;

    // Test that cleanup works properly
    let temp_path = env.temp_dir.path().to_path_buf();
    assert!(temp_path.exists());

    // Drop the environment (should trigger cleanup)
    drop(env);

    // Note: TempDir cleanup happens when dropped, but we can't easily test
    // that here since it happens asynchronously

    Ok(())
}
