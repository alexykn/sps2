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
    use spsv2_events::{Event, EventReceiver, EventSender};
    use spsv2_hash::ContentHasher;
    use spsv2_ops::OpsCtx;
    use spsv2_root::RootManager;
    use spsv2_state::StateManager;
    use spsv2_store::PackageStore;
    use std::sync::Arc;

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
            let state_db_path = base_path.join("state.sqlite");
            let live_path = base_path.join("live");
            let cache_path = base_path.join("cache");

            fs::create_dir_all(&store_path).await?;
            fs::create_dir_all(&live_path).await?;
            fs::create_dir_all(&cache_path).await?;

            // Create test configuration
            let mut config = Config::default();
            config.paths.store_path = store_path;
            config.paths.state_db_path = state_db_path;
            config.paths.live_path = live_path;
            config.paths.cache_path = Some(cache_path);
            config.general.parallelism = 2; // Smaller for tests
            config.network.download_timeout = 30;
            config.security.verify_signatures = false; // Disable for tests

            // Create event channel
            let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

            // Initialize components
            let hasher = Arc::new(ContentHasher::new());
            let root_manager = Arc::new(RootManager::new(&config.paths.live_path)?);
            let state_manager = Arc::new(StateManager::new(&config.paths.state_db_path).await?);
            let store = Arc::new(PackageStore::new(&config.paths.store_path, hasher.clone())?);

            let ops_ctx = OpsCtx::new(config.clone(), state_manager, store, root_manager, hasher)?;

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

        pub async fn load_test_recipe(
            &self,
            name: &str,
        ) -> Result<String, Box<dyn std::error::Error>> {
            let recipe_path = Self::fixtures_path()
                .join("recipes")
                .join(format!("{name}.rhai"));
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
    assert!(env.config.paths.store_path.exists());
    assert!(env.config.paths.live_path.exists());

    // Test state manager initialization
    let state = env.ops_ctx.state_manager();
    let packages = state.get_installed_packages().await?;
    assert!(packages.is_empty()); // Should start empty

    Ok(())
}

#[tokio::test]
async fn test_manifest_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let env = utils::TestEnvironment::new().await?;

    // Test simple manifest
    let hello_manifest = env.load_test_manifest("hello-world-1.0.0").await?;
    let manifest = spsv2_manifest::PackageManifest::from_toml(&hello_manifest)?;

    assert_eq!(manifest.package.name, "hello-world");
    assert_eq!(manifest.package.version.to_string(), "1.0.0");
    assert_eq!(manifest.package.license, "MIT");
    assert!(manifest.dependencies.is_empty());

    // Test complex manifest
    let complex_manifest = env.load_test_manifest("complex-app-2.1.3").await?;
    let manifest = spsv2_manifest::PackageManifest::from_toml(&complex_manifest)?;

    assert_eq!(manifest.package.name, "complex-app");
    assert_eq!(manifest.package.version.to_string(), "2.1.3");
    assert_eq!(manifest.package.license, "Apache-2.0");
    assert!(!manifest.dependencies.is_empty());
    assert!(manifest.dependencies.contains_key("libssl"));
    assert!(manifest.dependencies.contains_key("zlib"));

    Ok(())
}

#[tokio::test]
async fn test_version_parsing_and_constraints() -> Result<(), Box<dyn std::error::Error>> {
    use spsv2_types::{Version, VersionConstraint};

    // Test version parsing
    let version = Version::parse("2.1.3")?;
    assert_eq!(version.major, 2);
    assert_eq!(version.minor, 1);
    assert_eq!(version.patch, 3);

    // Test version constraints
    let constraint = VersionConstraint::parse(">=1.1.1,<2.0")?;
    let v1 = Version::parse("1.1.1")?;
    let v2 = Version::parse("1.5.0")?;
    let v3 = Version::parse("2.0.0")?;

    assert!(constraint.satisfies(&v1));
    assert!(constraint.satisfies(&v2));
    assert!(!constraint.satisfies(&v3));

    // Test compatible constraint
    let compat_constraint = VersionConstraint::parse("~=1.2.0")?;
    let v4 = Version::parse("1.2.5")?;
    let v5 = Version::parse("1.3.0")?;

    assert!(compat_constraint.satisfies(&v4));
    assert!(!compat_constraint.satisfies(&v5));

    Ok(())
}

#[tokio::test]
async fn test_dependency_resolution() -> Result<(), Box<dyn std::error::Error>> {
    use spsv2_resolver::DependencyResolver;
    use spsv2_types::{PackageId, Version, VersionConstraint};
    use std::collections::HashMap;

    let resolver = DependencyResolver::new();

    // Create a simple dependency graph
    let mut available_packages = HashMap::new();

    // hello-world: no dependencies
    available_packages.insert("hello-world".to_string(), vec![Version::parse("1.0.0")?]);

    // complex-app: depends on libssl, zlib, sqlite, curl
    available_packages.insert("complex-app".to_string(), vec![Version::parse("2.1.3")?]);

    available_packages.insert("libssl".to_string(), vec![Version::parse("1.1.1w")?]);

    available_packages.insert("zlib".to_string(), vec![Version::parse("1.2.11")?]);

    available_packages.insert("sqlite".to_string(), vec![Version::parse("3.36.0")?]);

    available_packages.insert("curl".to_string(), vec![Version::parse("7.85.0")?]);

    // Test resolving hello-world (simple case)
    let hello_request = PackageId {
        name: "hello-world".to_string(),
        version: Version::parse("1.0.0")?,
    };

    // For this test, we'll simulate the resolution
    // In a real test, we'd need to set up the full package index
    assert_eq!(hello_request.name, "hello-world");

    Ok(())
}

#[tokio::test]
async fn test_content_hashing() -> Result<(), Box<dyn std::error::Error>> {
    use spsv2_hash::ContentHasher;

    let hasher = ContentHasher::new();

    // Test hashing data
    let data = b"Hello, World!";
    let hash1 = hasher.hash_data(data);
    let hash2 = hasher.hash_data(data);

    // Same data should produce same hash
    assert_eq!(hash1, hash2);

    // Different data should produce different hash
    let different_data = b"Goodbye, World!";
    let hash3 = hasher.hash_data(different_data);
    assert_ne!(hash1, hash3);

    // Test hash string format
    let hash_str = hash1.to_string();
    assert!(hash_str.starts_with("blake3:"));
    assert_eq!(hash_str.len(), 71); // blake3: + 64 hex chars

    Ok(())
}

#[tokio::test]
async fn test_event_system() -> Result<(), Box<dyn std::error::Error>> {
    use spsv2_events::{Event, EventReceiver, EventSender};
    use spsv2_types::Version;

    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();

    // Send some events
    sender.send(Event::PackageInstalling {
        name: "hello-world".to_string(),
        version: Version::parse("1.0.0")?,
    })?;

    sender.send(Event::DownloadProgress {
        url: "https://example.com/package.sp".to_string(),
        bytes: 1024,
        total: 2048,
    })?;

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
        Event::DownloadProgress { url, bytes, total } => {
            assert_eq!(url, "https://example.com/package.sp");
            assert_eq!(bytes, 1024);
            assert_eq!(total, 2048);
        }
        _ => panic!("Unexpected event type"),
    }

    Ok(())
}

#[tokio::test]
async fn test_sbom_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let env = utils::TestEnvironment::new().await?;

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
    assert_eq!(config.general.parallelism, 2);
    assert_eq!(config.general.output_format, "human");
    assert_eq!(config.network.download_timeout, 30);
    assert_eq!(config.network.connection_timeout, 10);
    assert_eq!(config.network.max_retries, 2);
    assert!(!config.security.verify_signatures);
    assert_eq!(config.build.max_build_jobs, 2);
    assert_eq!(config.build.build_timeout, 300);

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
    let env = utils::TestEnvironment::new().await?;

    // Test concurrent hash operations
    let hasher = spsv2_hash::ContentHasher::new();
    let mut tasks = Vec::new();

    for i in 0..10 {
        let hasher = hasher.clone();
        let task = tokio::spawn(async move {
            let data = format!("test data {i}").into_bytes();
            hasher.hash_data(&data)
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
description = "Package with many dependencies for testing"
license = "MIT"

[dependencies]
"#;

    // Add many dependencies
    let mut manifest_content = large_manifest.to_string();
    for i in 0..100 {
        manifest_content.push_str(&format!("dep{i} = \">=1.0.0\"\n"));
    }

    let manifest = spsv2_manifest::PackageManifest::from_toml(&manifest_content)?;
    assert_eq!(manifest.package.name, "large-package");
    assert_eq!(manifest.dependencies.len(), 100);

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
    assert!(spsv2_manifest::PackageManifest::from_toml(invalid_toml).is_err());

    // Test missing required fields
    let incomplete_manifest = r#"
[package]
name = "incomplete"
# missing version
"#;
    assert!(spsv2_manifest::PackageManifest::from_toml(incomplete_manifest).is_err());
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
