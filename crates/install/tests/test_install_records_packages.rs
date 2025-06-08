//! Integration test to verify that packages are recorded in states during installation

use sps2_events::EventSender;
use sps2_index::{DependencyInfo, Index, IndexManager, VersionEntry};
use sps2_install::{AtomicInstaller, InstallContext};
use sps2_resolver::{PackageId, Resolver};
use sps2_state::StateManager;
use sps2_store::PackageStore;
use sps2_types::{PackageSpec, Version};
use std::collections::HashMap;
use tempfile::tempdir;

/// Create a minimal valid package in the store for testing
async fn create_test_package_in_store(
    store_path: &std::path::Path,
    name: &str,
    version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create package directory structure
    tokio::fs::create_dir_all(store_path).await?;
    tokio::fs::create_dir_all(store_path.join("files").join("bin")).await?;

    // Create manifest
    let manifest_content = format!(
        r#"
[package]
name = "{}"
version = "{}"
revision = 1
arch = "arm64"
description = "Test package"

[dependencies]
"#,
        name, version
    );

    tokio::fs::write(store_path.join("manifest.toml"), manifest_content).await?;

    // Create a dummy binary
    tokio::fs::write(
        store_path.join("files").join("bin").join(name),
        format!("#!/bin/sh\necho '{}'", name).as_bytes(),
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn test_install_records_new_packages_in_state() {
    let temp = tempdir().unwrap();

    // Set up index with a test package
    let mut index_manager = IndexManager::new(temp.path());
    let mut index = Index::new();

    let test_entry = VersionEntry {
        revision: 1,
        arch: "arm64".to_string(),
        blake3: "test_hash".to_string(),
        download_url: "https://example.com/hello-1.0.0.sp".to_string(),
        minisig_url: "https://example.com/hello-1.0.0.sp.minisig".to_string(),
        dependencies: DependencyInfo::default(),
        sbom: None,
        description: Some("Hello package".to_string()),
        homepage: None,
        license: None,
    };

    index.add_version("hello".to_string(), "1.0.0".to_string(), test_entry);

    let json = index.to_json().unwrap();
    index_manager.load(Some(&json)).await.unwrap();

    // Set up other components
    let _resolver = Resolver::new(index_manager);
    let state_manager = StateManager::new(temp.path()).await.unwrap();
    let store = PackageStore::new(temp.path().to_path_buf());

    // Create the test package in store
    let package_id = PackageId::new("hello".to_string(), Version::parse("1.0.0").unwrap());
    let store_path = store
        .get_package_path(&package_id.name, &package_id.version)
        .unwrap();
    create_test_package_in_store(&store_path, "hello", "1.0.0")
        .await
        .unwrap();

    // Create atomic installer
    let mut atomic_installer = AtomicInstaller::new(state_manager.clone(), store.clone())
        .await
        .unwrap();

    // Create install context
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let event_sender: EventSender = tx;

    let context = InstallContext::new()
        .add_package(PackageSpec::parse("hello>=1.0.0").unwrap())
        .with_event_sender(event_sender);

    // Create resolved packages (simulating that resolution has already happened)
    let mut resolved_packages = HashMap::new();
    resolved_packages.insert(
        package_id.clone(),
        sps2_resolver::ResolvedNode {
            name: package_id.name.clone(),
            version: package_id.version.clone(),
            action: sps2_resolver::NodeAction::Download,
            deps: vec![],
            url: Some("https://example.com/hello-1.0.0.sp".to_string()),
            path: Some(store_path),
        },
    );

    // Get initial state
    let initial_state_id = state_manager.get_current_state_id().await.unwrap();
    let initial_packages = state_manager
        .get_installed_packages_in_state(&initial_state_id)
        .await
        .unwrap();

    // Should start with no packages
    assert_eq!(initial_packages.len(), 0);

    // Perform installation
    let result = atomic_installer
        .install(&context, &resolved_packages, None)
        .await
        .unwrap();

    // Verify installation result
    assert_eq!(result.installed_packages.len(), 1);
    assert!(result.installed_packages.contains(&package_id));

    // Get new state
    let new_state_id = state_manager.get_current_state_id().await.unwrap();
    assert_ne!(new_state_id, initial_state_id);

    // Verify the package is recorded in the new state
    let installed_packages = state_manager
        .get_installed_packages_in_state(&new_state_id)
        .await
        .unwrap();

    assert_eq!(installed_packages.len(), 1);
    assert_eq!(installed_packages[0].name, "hello");
    assert_eq!(installed_packages[0].version, "1.0.0");

    // The hash will be "placeholder-hash" until proper hash tracking is implemented
    assert_eq!(installed_packages[0].hash, "placeholder-hash");
}

#[tokio::test]
async fn test_consecutive_installs_preserve_packages() {
    let temp = tempdir().unwrap();

    // Set up index with two packages
    let mut index_manager = IndexManager::new(temp.path());
    let mut index = Index::new();

    for (name, version) in [("first", "1.0.0"), ("second", "2.0.0")] {
        let entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: format!("{}_hash", name),
            download_url: format!("https://example.com/{}-{}.sp", name, version),
            minisig_url: format!("https://example.com/{}-{}.sp.minisig", name, version),
            dependencies: DependencyInfo::default(),
            sbom: None,
            description: Some(format!("{} package", name)),
            homepage: None,
            license: None,
        };
        index.add_version(name.to_string(), version.to_string(), entry);
    }

    let json = index.to_json().unwrap();
    index_manager.load(Some(&json)).await.unwrap();

    let _resolver = Resolver::new(index_manager);
    let state_manager = StateManager::new(temp.path()).await.unwrap();
    let store = PackageStore::new(temp.path().to_path_buf());

    // Create both packages in store
    for (name, version) in [("first", "1.0.0"), ("second", "2.0.0")] {
        let version_obj = Version::parse(version).unwrap();
        let store_path = store.get_package_path(name, &version_obj).unwrap();
        create_test_package_in_store(&store_path, name, version)
            .await
            .unwrap();
    }

    let mut atomic_installer = AtomicInstaller::new(state_manager.clone(), store.clone())
        .await
        .unwrap();

    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let event_sender: EventSender = tx;

    // First installation
    let package_id1 = PackageId::new("first".to_string(), Version::parse("1.0.0").unwrap());
    let context1 = InstallContext::new()
        .add_package(PackageSpec::parse("first>=1.0.0").unwrap())
        .with_event_sender(event_sender.clone());

    let mut resolved_packages1 = HashMap::new();
    resolved_packages1.insert(
        package_id1.clone(),
        sps2_resolver::ResolvedNode {
            name: package_id1.name.clone(),
            version: package_id1.version.clone(),
            action: sps2_resolver::NodeAction::Download,
            deps: vec![],
            url: Some("https://example.com/first-1.0.0.sp".to_string()),
            path: Some(
                store
                    .get_package_path(&package_id1.name, &package_id1.version)
                    .unwrap(),
            ),
        },
    );

    let _result1 = atomic_installer
        .install(&context1, &resolved_packages1, None)
        .await
        .unwrap();

    // Verify first package is installed
    let state_after_first = state_manager.get_current_state_id().await.unwrap();
    let packages_after_first = state_manager
        .get_installed_packages_in_state(&state_after_first)
        .await
        .unwrap();

    assert_eq!(packages_after_first.len(), 1);
    assert_eq!(packages_after_first[0].name, "first");

    // Second installation
    let package_id2 = PackageId::new("second".to_string(), Version::parse("2.0.0").unwrap());
    let context2 = InstallContext::new()
        .add_package(PackageSpec::parse("second>=2.0.0").unwrap())
        .with_event_sender(event_sender);

    let mut resolved_packages2 = HashMap::new();
    resolved_packages2.insert(
        package_id2.clone(),
        sps2_resolver::ResolvedNode {
            name: package_id2.name.clone(),
            version: package_id2.version.clone(),
            action: sps2_resolver::NodeAction::Download,
            deps: vec![],
            url: Some("https://example.com/second-2.0.0.sp".to_string()),
            path: Some(
                store
                    .get_package_path(&package_id2.name, &package_id2.version)
                    .unwrap(),
            ),
        },
    );

    let result2 = atomic_installer
        .install(&context2, &resolved_packages2, None)
        .await
        .unwrap();

    assert_eq!(result2.installed_packages.len(), 1);

    // Verify both packages are in the final state
    let final_state_id = state_manager.get_current_state_id().await.unwrap();
    assert_ne!(final_state_id, state_after_first);

    let final_packages = state_manager
        .get_installed_packages_in_state(&final_state_id)
        .await
        .unwrap();

    // This is the key assertion - both packages should be present
    assert_eq!(
        final_packages.len(),
        2,
        "Both packages should be in the final state"
    );

    let package_names: Vec<String> = final_packages.iter().map(|p| p.name.clone()).collect();
    assert!(
        package_names.contains(&"first".to_string()),
        "First package should still be present"
    );
    assert!(
        package_names.contains(&"second".to_string()),
        "Second package should be present"
    );
}
