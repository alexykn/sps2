//! Tests for virtual environment cleanup during uninstall

use sps2_errors::Error;
use sps2_events::Event;
use sps2_index::{Index, IndexManager};
use sps2_install::{UninstallContext, UninstallOperation};
use sps2_resolver::PackageId;
use sps2_state::{models::PackageRef, StateManager};
use sps2_store::PackageStore;
use sps2_types::Version;
use tempfile::tempdir;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_venv_cleanup_on_uninstall() -> Result<(), Error> {
    let temp_dir = tempdir().unwrap();
    let base_path = temp_dir.path();

    // Create necessary directories
    tokio::fs::create_dir_all(base_path.join("states"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(base_path.join("live"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(base_path.join("venvs"))
        .await
        .unwrap();

    // Add initial content to live directory
    tokio::fs::write(base_path.join("live").join("initial.txt"), b"initial state")
        .await
        .unwrap();

    // Initialize components
    let state_manager = StateManager::new(base_path).await?;
    let store = PackageStore::new(base_path.to_path_buf());

    // Create index manager with empty index
    let mut index_manager = IndexManager::new(base_path);
    let index = Index::new();
    let json = index.to_json().unwrap();
    index_manager.load(Some(&json)).await.unwrap();

    // Setup a Python package in the state
    let package_id = PackageId {
        name: "pytest".to_string(),
        version: Version::parse("7.4.0").unwrap(),
    };

    // Add the package to the current state
    let package_ref = PackageRef {
        state_id: state_manager.get_current_state_id().await?,
        package_id: package_id.clone(),
        hash: "abcd1234".to_string(),
        size: 1024,
    };

    // Create a fake venv directory
    let venv_path = base_path.join("venvs").join("pytest-7.4.0");
    tokio::fs::create_dir_all(&venv_path).await?;
    tokio::fs::write(venv_path.join("pyvenv.cfg"), b"test venv").await?;

    // Add package with venv path
    state_manager
        .add_package_ref_with_venv(&package_ref, Some(&venv_path.display().to_string()))
        .await?;

    // Verify venv exists before uninstall
    assert!(venv_path.exists());

    // Create event channel to capture events
    let (tx, mut rx) = mpsc::unbounded_channel();

    // Create uninstall operation
    let mut uninstall_op = UninstallOperation::new(state_manager.clone(), store);

    // Create uninstall context
    let context = UninstallContext::new()
        .add_package("pytest".to_string())
        .with_event_sender(tx);

    // Execute uninstall
    let result = uninstall_op.execute(context).await?;

    // Verify package was removed
    assert_eq!(result.removed_packages.len(), 1);
    assert_eq!(result.removed_packages[0].name, "pytest");

    // Verify venv was removed
    assert!(!venv_path.exists(), "Venv directory should be removed");

    // Check that we received the correct events
    let mut found_removing_event = false;
    let mut found_removed_event = false;

    while let Ok(event) = rx.try_recv() {
        match event {
            Event::PythonVenvRemoving {
                package,
                version,
                venv_path: path,
            } => {
                assert_eq!(package, "pytest");
                assert_eq!(version, Version::parse("7.4.0").unwrap());
                assert!(path.contains("pytest-7.4.0"));
                found_removing_event = true;
            }
            Event::PythonVenvRemoved {
                package,
                version,
                venv_path: path,
            } => {
                assert_eq!(package, "pytest");
                assert_eq!(version, Version::parse("7.4.0").unwrap());
                assert!(path.contains("pytest-7.4.0"));
                found_removed_event = true;
            }
            _ => {}
        }
    }

    assert!(found_removing_event, "Should emit PythonVenvRemoving event");
    assert!(found_removed_event, "Should emit PythonVenvRemoved event");

    // Verify venv path is cleared from database
    let venv_path_after = state_manager
        .get_package_venv_path("pytest", "7.4.0")
        .await?;
    assert_eq!(venv_path_after, None);

    Ok(())
}

#[tokio::test]
async fn test_non_python_package_uninstall() -> Result<(), Error> {
    let temp_dir = tempdir().unwrap();
    let base_path = temp_dir.path();

    // Create necessary directories
    tokio::fs::create_dir_all(base_path.join("states"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(base_path.join("live"))
        .await
        .unwrap();

    // Add initial content to live directory
    tokio::fs::write(base_path.join("live").join("initial.txt"), b"initial state")
        .await
        .unwrap();

    // Initialize components
    let state_manager = StateManager::new(base_path).await?;
    let store = PackageStore::new(base_path.to_path_buf());

    // Setup a non-Python package (no venv)
    let package_id = PackageId {
        name: "curl".to_string(),
        version: Version::parse("8.0.0").unwrap(),
    };

    // Add the package to the current state
    let package_ref = PackageRef {
        state_id: state_manager.get_current_state_id().await?,
        package_id: package_id.clone(),
        hash: "efgh5678".to_string(),
        size: 2048,
    };

    // Add package without venv path
    state_manager.add_package_ref(&package_ref).await?;

    // Create event channel
    let (tx, mut rx) = mpsc::unbounded_channel();

    // Create uninstall operation
    let mut uninstall_op = UninstallOperation::new(state_manager.clone(), store);

    // Create uninstall context
    let context = UninstallContext::new()
        .add_package("curl".to_string())
        .with_event_sender(tx);

    // Execute uninstall
    let result = uninstall_op.execute(context).await?;

    // Verify package was removed
    assert_eq!(result.removed_packages.len(), 1);
    assert_eq!(result.removed_packages[0].name, "curl");

    // Verify no venv events were emitted
    let mut found_venv_event = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            Event::PythonVenvRemoving { .. } | Event::PythonVenvRemoved { .. } => {
                found_venv_event = true;
            }
            _ => {}
        }
    }

    assert!(
        !found_venv_event,
        "Should not emit venv events for non-Python packages"
    );

    Ok(())
}
