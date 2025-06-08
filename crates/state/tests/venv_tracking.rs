//! Tests for Python virtual environment tracking functionality

use sps2_resolver::PackageId;
use sps2_state::{models::PackageRef, StateManager};
use sps2_types::Version;
use tempfile::tempdir;

#[tokio::test]
async fn test_venv_tracking() {
    let temp = tempdir().unwrap();

    // Create necessary directories
    tokio::fs::create_dir_all(temp.path().join("states"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(temp.path().join("live"))
        .await
        .unwrap();

    // Add initial content to live directory
    tokio::fs::write(
        temp.path().join("live").join("initial.txt"),
        b"initial state",
    )
    .await
    .unwrap();

    let state_manager = StateManager::new(temp.path()).await.unwrap();

    // Create a package reference for a Python package
    let package_id = PackageId {
        name: "pytest".to_string(),
        version: Version::parse("7.4.0").unwrap(),
    };

    let package_ref = PackageRef {
        state_id: state_manager.get_current_state_id().await.unwrap(),
        package_id,
        hash: "abcd1234".to_string(),
        size: 1024,
    };

    // Add package with venv path
    let venv_path = "/opt/pm/venvs/pytest-7.4.0";
    state_manager
        .add_package_ref_with_venv(&package_ref, Some(venv_path))
        .await
        .unwrap();

    // Verify we can retrieve the venv path
    let retrieved_path = state_manager
        .get_package_venv_path("pytest", "7.4.0")
        .await
        .unwrap();

    assert_eq!(retrieved_path, Some(venv_path.to_string()));

    // Test getting all packages with venvs
    let packages_with_venvs = state_manager.get_packages_with_venvs().await.unwrap();
    assert_eq!(packages_with_venvs.len(), 1);
    assert_eq!(packages_with_venvs[0].0, "pytest");
    assert_eq!(packages_with_venvs[0].1, "7.4.0");
    assert_eq!(packages_with_venvs[0].2, venv_path);

    // Test updating venv path
    let new_venv_path = "/opt/pm/venvs/pytest-7.4.0-updated";
    state_manager
        .update_package_venv_path("pytest", "7.4.0", Some(new_venv_path))
        .await
        .unwrap();

    let updated_path = state_manager
        .get_package_venv_path("pytest", "7.4.0")
        .await
        .unwrap();

    assert_eq!(updated_path, Some(new_venv_path.to_string()));

    // Test removing venv path
    state_manager
        .update_package_venv_path("pytest", "7.4.0", None)
        .await
        .unwrap();

    let removed_path = state_manager
        .get_package_venv_path("pytest", "7.4.0")
        .await
        .unwrap();

    assert_eq!(removed_path, None);
}

#[tokio::test]
async fn test_package_without_venv() {
    let temp = tempdir().unwrap();

    // Create necessary directories
    tokio::fs::create_dir_all(temp.path().join("states"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(temp.path().join("live"))
        .await
        .unwrap();

    // Add initial content to live directory
    tokio::fs::write(
        temp.path().join("live").join("initial.txt"),
        b"initial state",
    )
    .await
    .unwrap();

    let state_manager = StateManager::new(temp.path()).await.unwrap();

    // Create a package reference for a non-Python package
    let package_id = PackageId {
        name: "curl".to_string(),
        version: Version::parse("8.0.0").unwrap(),
    };

    let package_ref = PackageRef {
        state_id: state_manager.get_current_state_id().await.unwrap(),
        package_id,
        hash: "efgh5678".to_string(),
        size: 2048,
    };

    // Add package without venv path
    state_manager.add_package_ref(&package_ref).await.unwrap();

    // Verify venv path is None
    let venv_path = state_manager
        .get_package_venv_path("curl", "8.0.0")
        .await
        .unwrap();

    assert_eq!(venv_path, None);

    // Verify it doesn't appear in packages with venvs
    let packages_with_venvs = state_manager.get_packages_with_venvs().await.unwrap();
    assert_eq!(packages_with_venvs.len(), 0);
}
