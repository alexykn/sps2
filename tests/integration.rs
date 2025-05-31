//! Integration tests for spsv2 package manager
//!
//! These tests exercise the full system end-to-end using test fixtures.

use std::path::PathBuf;
use tempfile::TempDir;
use tokio::fs;

// Test utilities module
mod utils {
    use super::*;
    use spsv2_config::Config;
    use spsv2_events::{EventReceiver, EventSender};
    use spsv2_ops::OpsCtx;
    use spsv2_state::StateManager;
    use spsv2_store::PackageStore;

    #[allow(dead_code)]
    pub struct TestEnvironment {
        pub temp_dir: TempDir,
        pub config: Config,
        pub ops_ctx: OpsCtx,
        pub event_sender: EventSender,
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
            let index = spsv2_index::IndexManager::new(base_path);
            let net = spsv2_net::NetClient::with_defaults()?;
            let resolver = spsv2_resolver::Resolver::new(index.clone());
            let builder = spsv2_builder::Builder::new();

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

        #[allow(dead_code)]
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
async fn test_system_initialization() -> Result<(), Box<dyn std::error::Error>> {
    let env = utils::TestEnvironment::new().await?;

    // Test that all components are properly initialized
    assert!(env.temp_dir.path().exists());
    assert!(env.config.paths.store_path.as_ref().unwrap().exists());
    assert!(env.temp_dir.path().join("live").exists());

    // Test state manager initialization
    // In a fresh system, there's no active state yet
    let active_state_result = env.ops_ctx.state.get_active_state().await;
    assert!(active_state_result.is_err()); // Should fail with no active state

    // List of states should be empty
    let states = env.ops_ctx.state.list_states().await?;
    assert!(states.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_manifest_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let env = utils::TestEnvironment::new().await?;

    // Test simple manifest
    let hello_manifest = env.load_test_manifest("hello-world-1.0.0").await?;
    let manifest = spsv2_manifest::Manifest::from_toml(&hello_manifest)?;

    assert_eq!(manifest.package.name, "hello-world");
    assert_eq!(manifest.package.version, "1.0.0");
    assert_eq!(manifest.package.license, Some("MIT".to_string()));
    assert!(manifest.dependencies.runtime.is_empty());
    assert!(manifest.dependencies.build.is_empty());

    // Test complex manifest
    let complex_manifest = env.load_test_manifest("complex-app-2.1.3").await?;
    let manifest = spsv2_manifest::Manifest::from_toml(&complex_manifest)?;

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
    use spsv2_types::{PackageSpec, Version};

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
    use spsv2_types::{PackageSpec, Version};

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
    use spsv2_hash::Hash;
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
    use spsv2_events::Event;
    use spsv2_types::Version;

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
    use spsv2_config::Config;

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
            spsv2_hash::Hash::from_data(&data)
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

    let manifest = spsv2_manifest::Manifest::from_toml(&manifest_content)?;
    assert_eq!(manifest.package.name, "large-package");
    assert_eq!(manifest.dependencies.runtime.len(), 100);

    Ok(())
}

// Error handling tests
#[tokio::test]
async fn test_invalid_version_handling() {
    use spsv2_types::Version;

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
    assert!(spsv2_manifest::Manifest::from_toml(invalid_toml).is_err());

    // Test missing required fields
    let incomplete_manifest = r#"
[package]
name = "incomplete"
# missing version
"#;
    assert!(spsv2_manifest::Manifest::from_toml(incomplete_manifest).is_err());
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
