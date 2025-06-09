//! Integration tests for content-addressable storage flow

use sps2_install::{InstallContext, InstallOperation};
use sps2_resolver::Resolver;
use sps2_state::StateManager;
use sps2_store::PackageStore;
use sps2_types::PackageSpec;
use tempfile::tempdir;

#[tokio::test]
async fn test_content_addressable_storage_flow() {
    // Create temporary directory for test
    let temp = tempdir().unwrap();
    let base_path = temp.path();

    // Initialize components
    let store = PackageStore::new(base_path.join("store"));
    let state_manager = StateManager::new(base_path).await.unwrap();

    // Create a minimal index for testing
    let mut index_manager = sps2_index::IndexManager::new(base_path);
    let mut index = sps2_index::Index::new();

    // Add a test package to the index
    let test_package = sps2_index::PackageEntry {
        versions: {
            let mut versions = std::collections::HashMap::new();
            versions.insert(
                "1.0.0".to_string(),
                sps2_index::VersionEntry {
                    revision: 1,
                    arch: "arm64".to_string(),
                    blake3: "test-hash".to_string(),
                    download_url: "https://example.com/test-pkg-1.0.0.sp".to_string(),
                    minisig_url: "https://example.com/test-pkg-1.0.0.sp.minisig".to_string(),
                    dependencies: sps2_index::DependencyInfo {
                        runtime: vec![],
                        build: vec![],
                    },
                    sbom: None,
                    description: Some("Test package".to_string()),
                    homepage: None,
                    license: None,
                },
            );
            versions
        },
    };

    index.packages.insert("test-pkg".to_string(), test_package);
    let index_json = index.to_json().unwrap();
    index_manager.load(Some(&index_json)).await.unwrap();

    let resolver = Resolver::new(index_manager);

    // Create install operation
    let _install_op =
        InstallOperation::new(resolver, state_manager.clone(), store.clone()).unwrap();

    // Create install context
    let _context =
        InstallContext::new().add_package(PackageSpec::parse("test-pkg==1.0.0").unwrap());

    // Note: This test would need a mock HTTP server to actually download packages
    // For now, we're testing the setup and flow

    // Verify store is empty initially
    let packages = store.list_packages().await.unwrap();
    assert_eq!(packages.len(), 0, "Store should be empty initially");

    // Verify package_map is empty initially
    let package_hash = state_manager
        .get_package_hash("test-pkg", "1.0.0")
        .await
        .unwrap();
    assert_eq!(
        package_hash, None,
        "Package should not be in package_map initially"
    );
}

#[tokio::test]
async fn test_flat_store_structure() {
    // Test that packages are stored in flat structure /opt/pm/store/<hash>/
    let temp = tempdir().unwrap();
    let store = PackageStore::new(temp.path().to_path_buf());

    // Create a fake hash
    let hash = sps2_hash::Hash::from_data(b"test content");

    // Get the package path
    let package_path = store.package_path(&hash);

    // Verify it's a flat structure (no sharding)
    let hash_hex = hash.to_hex();
    assert!(
        package_path.ends_with(&hash_hex),
        "Package path should end with full hash"
    );

    // Verify no intermediate directories (no 2-char prefix)
    let relative_path = package_path.strip_prefix(temp.path()).unwrap();
    let components: Vec<_> = relative_path.components().collect();
    assert_eq!(
        components.len(),
        1,
        "Should have only one path component (the hash)"
    );
}

#[tokio::test]
async fn test_package_map_population() {
    // Test that package_map is populated when packages are added
    let temp = tempdir().unwrap();
    let _store = PackageStore::new(temp.path().join("store"));
    let state_manager = StateManager::new(temp.path()).await.unwrap();

    // Add a package to package_map
    let package_name = "test-pkg";
    let package_version = "1.0.0";
    let hash = sps2_hash::Hash::from_data(b"test content");

    state_manager
        .add_package_map(package_name, package_version, &hash.to_hex())
        .await
        .unwrap();

    // Verify it can be retrieved
    let retrieved_hash = state_manager
        .get_package_hash(package_name, package_version)
        .await
        .unwrap();

    assert_eq!(
        retrieved_hash,
        Some(hash.to_hex()),
        "Package hash should be retrievable from package_map"
    );
}

#[tokio::test]
async fn test_store_list_packages_flat() {
    // Test that list_packages works with flat structure
    let temp = tempdir().unwrap();
    let store = PackageStore::new(temp.path().to_path_buf());

    // Create some package directories with hash names
    let hash1 = sps2_hash::Hash::from_data(b"package 1");
    let hash2 = sps2_hash::Hash::from_data(b"package 2");

    let path1 = store.package_path(&hash1);
    let path2 = store.package_path(&hash2);

    tokio::fs::create_dir_all(&path1).await.unwrap();
    tokio::fs::create_dir_all(&path2).await.unwrap();

    // List packages
    let mut packages = store.list_packages().await.unwrap();
    packages.sort_by_key(|h| h.to_hex());

    assert_eq!(packages.len(), 2, "Should find 2 packages");

    let mut expected = vec![hash1, hash2];
    expected.sort_by_key(|h| h.to_hex());

    assert_eq!(packages, expected, "Should find the correct hashes");
}
