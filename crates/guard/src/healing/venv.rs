//! Python virtual environment healing logic

use sps2_errors::{Error, OpsError};
use sps2_events::{Event, EventSender};
use sps2_hash::Hash;
use sps2_state::{queries, StateManager};
use sps2_store::{PackageStore, StoredPackage};
use std::collections::HashMap;
use std::path::PathBuf;

/// Heal a missing Python virtual environment by recreating it
///
/// # Errors
///
/// Returns an error if:
/// - Package information cannot be retrieved
/// - Python metadata is missing
/// - Venv recreation fails
/// - Package reinstallation fails
pub async fn heal_missing_venv(
    state_manager: &StateManager,
    store: &PackageStore,
    tx: &EventSender,
    package_name: &str,
    package_version: &str,
    venv_path: &str,
) -> Result<(), Error> {
    let _ = tx.send(Event::DebugLog {
        message: format!(
            "Starting venv healing for {package_name}-{package_version} at {venv_path}"
        ),
        context: HashMap::default(),
    });

    // Step 1: Get package information from database
    let mut state_tx = state_manager.begin_transaction().await?;
    let state_id = state_manager.get_active_state().await?;
    let packages = queries::get_state_packages(&mut state_tx, &state_id).await?;
    state_tx.commit().await?;

    let package = packages
        .iter()
        .find(|p| p.name == package_name && p.version == package_version)
        .ok_or_else(|| OpsError::OperationFailed {
            message: format!("Package {package_name}-{package_version} not found in state"),
        })?;

    // Step 2: Load package manifest from store to get Python metadata
    let package_hash = Hash::from_hex(&package.hash).map_err(|e| OpsError::OperationFailed {
        message: format!("Invalid package hash: {e}"),
    })?;
    let store_path = store.package_path(&package_hash);

    if !store_path.exists() {
        return Err(OpsError::OperationFailed {
            message: format!(
                "Package content missing from store for {package_name}-{package_version}"
            ),
        }
        .into());
    }

    let stored_package = StoredPackage::load(&store_path).await?;
    let manifest = stored_package.manifest();

    let python_metadata = manifest
        .python
        .as_ref()
        .ok_or_else(|| OpsError::OperationFailed {
            message: format!("Package {package_name}-{package_version} is not a Python package"),
        })?;

    // Step 3: Capture existing pip packages if venv partially exists
    let venv_path_buf = PathBuf::from(venv_path);
    let python_bin = venv_path_buf.join("bin/python");
    let mut existing_packages = Vec::new();

    if python_bin.exists() {
        let _ = tx.send(Event::DebugLog {
            message: format!("Capturing existing pip packages from {venv_path}"),
            context: HashMap::default(),
        });

        // Run pip freeze to capture existing packages
        match tokio::process::Command::new(&python_bin)
            .arg("-m")
            .arg("pip")
            .arg("freeze")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let freeze_output = String::from_utf8_lossy(&output.stdout);
                existing_packages = freeze_output
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(String::from)
                    .collect();

                let _ = tx.send(Event::DebugLog {
                    message: format!("Captured {} existing packages", existing_packages.len()),
                    context: HashMap::default(),
                });
            }
            _ => {
                let _ = tx.send(Event::DebugLog {
                    message: "Failed to capture existing packages, proceeding with fresh venv"
                        .to_string(),
                    context: HashMap::default(),
                });
            }
        }
    }

    // Step 4: Remove corrupted venv
    if venv_path_buf.exists() {
        tokio::fs::remove_dir_all(&venv_path_buf)
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to remove corrupted venv: {e}"),
            })?;
    }

    // Step 5: Create new venv using the PythonVenvManager
    let venvs_base = PathBuf::from("/opt/pm/venvs");
    let venv_manager = sps2_install::PythonVenvManager::new(venvs_base);

    let package_id = sps2_install::python::PackageId::new(
        package_name.to_string(),
        sps2_types::Version::parse(package_version).map_err(|e| OpsError::OperationFailed {
            message: format!("Invalid version: {e}"),
        })?,
    );

    venv_manager
        .create_venv(&package_id, python_metadata, Some(tx))
        .await?;

    // Step 6: Install the wheel file
    let wheel_path = stored_package
        .files_path()
        .join(&python_metadata.wheel_file);
    let requirements_path = stored_package
        .files_path()
        .join(&python_metadata.requirements_file);

    venv_manager
        .install_wheel(
            &package_id,
            &venv_path_buf,
            &wheel_path,
            Some(&requirements_path),
            Some(tx),
        )
        .await?;

    // Step 7: Restore previously installed packages (best effort)
    if !existing_packages.is_empty() {
        let _ = tx.send(Event::DebugLog {
            message: format!(
                "Attempting to restore {} previously installed packages",
                existing_packages.len()
            ),
            context: HashMap::default(),
        });

        // Create a temporary requirements file with the captured packages
        let temp_reqs = venv_path_buf.join("restore_requirements.txt");
        tokio::fs::write(&temp_reqs, existing_packages.join("\n"))
            .await
            .map_err(|e| OpsError::OperationFailed {
                message: format!("Failed to create restore requirements: {e}"),
            })?;

        // Try to reinstall the packages (don't fail if some packages can't be installed)
        let output = tokio::process::Command::new("uv")
            .arg("pip")
            .arg("install")
            .arg("--python")
            .arg(&python_bin)
            .arg("-r")
            .arg(&temp_reqs)
            .output()
            .await;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_reqs).await;

        match output {
            Ok(result) if result.status.success() => {
                let _ = tx.send(Event::DebugLog {
                    message: "Successfully restored previous packages".to_string(),
                    context: HashMap::default(),
                });
            }
            _ => {
                let _ = tx.send(Event::DebugLog {
                    message: "Some packages could not be restored, but venv is functional"
                        .to_string(),
                    context: HashMap::default(),
                });
            }
        }
    }

    let _ = tx.send(Event::DebugLog {
        message: format!("Successfully healed venv for {package_name}-{package_version}"),
        context: HashMap::default(),
    });

    Ok(())
}
