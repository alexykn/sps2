//! Python virtual environment management utilities

use sps2_errors::{Error, InstallError};
use sps2_events::{Event, EventSender};
use sps2_manifest::Manifest;
use sps2_resolver::PackageId;
use sps2_types::{PythonPackageMetadata, Version};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Python virtual environment manager
pub struct PythonVenvManager {
    /// Base path for venvs (/opt/pm/venvs)
    venvs_base: PathBuf,
}

impl PythonVenvManager {
    /// Create a new Python venv manager
    pub fn new(venvs_base: PathBuf) -> Self {
        Self { venvs_base }
    }

    /// Get the venv path for a package
    pub fn venv_path(&self, package_name: &str, version: &Version) -> PathBuf {
        self.venvs_base.join(format!("{package_name}-{version}"))
    }

    /// Create a virtual environment for a Python package
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - uv tool is not available
    /// - venv creation fails
    /// - directory creation fails
    pub async fn create_venv(
        &self,
        package_id: &PackageId,
        python_metadata: &PythonPackageMetadata,
        event_sender: Option<&EventSender>,
    ) -> Result<PathBuf, Error> {
        let venv_path = self.venv_path(&package_id.name, &package_id.version);

        // Send venv creating event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::PythonVenvCreating {
                package: package_id.name.clone(),
                version: package_id.version.clone(),
                venv_path: venv_path.display().to_string(),
            });
        }

        // Ensure parent directory exists
        if let Some(parent) = venv_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "create_venv_parent".to_string(),
                    path: parent.display().to_string(),
                    message: e.to_string(),
                })?;
        }

        // Remove existing venv if it exists
        if venv_path.exists() {
            tokio::fs::remove_dir_all(&venv_path).await.map_err(|e| {
                InstallError::FilesystemError {
                    operation: "remove_existing_venv".to_string(),
                    path: venv_path.display().to_string(),
                    message: e.to_string(),
                }
            })?;
        }

        // Create venv using uv
        let output = Command::new("uv")
            .arg("venv")
            .arg("--python")
            .arg(&python_metadata.requires_python)
            .arg(&venv_path)
            .output()
            .await
            .map_err(|e| InstallError::PythonVenvError {
                package: package_id.name.clone(),
                message: format!("Failed to execute uv: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(InstallError::PythonVenvError {
                package: package_id.name.clone(),
                message: format!("uv venv failed: {stderr}"),
            }
            .into());
        }

        // Send venv created event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::PythonVenvCreated {
                package: package_id.name.clone(),
                version: package_id.version.clone(),
                venv_path: venv_path.display().to_string(),
            });
        }

        Ok(venv_path)
    }

    /// Install a wheel file into the virtual environment
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The wheel file doesn't exist
    /// - uv pip install fails
    pub async fn install_wheel(
        &self,
        package_id: &PackageId,
        venv_path: &Path,
        wheel_path: &Path,
        requirements_path: Option<&Path>,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        // Send wheel installing event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::PythonWheelInstalling {
                package: package_id.name.clone(),
                version: package_id.version.clone(),
                wheel_file: wheel_path.display().to_string(),
            });
        }

        // Verify wheel exists
        if !wheel_path.exists() {
            return Err(InstallError::PythonVenvError {
                package: package_id.name.clone(),
                message: format!("Wheel file not found: {}", wheel_path.display()),
            }
            .into());
        }

        let python_bin = venv_path.join("bin/python");

        // Install requirements first if provided
        if let Some(reqs_path) = requirements_path {
            if reqs_path.exists() {
                let output = Command::new("uv")
                    .arg("pip")
                    .arg("install")
                    .arg("--python")
                    .arg(&python_bin)
                    .arg("-r")
                    .arg(reqs_path)
                    .output()
                    .await
                    .map_err(|e| InstallError::PythonVenvError {
                        package: package_id.name.clone(),
                        message: format!("Failed to install requirements: {e}"),
                    })?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(InstallError::PythonVenvError {
                        package: package_id.name.clone(),
                        message: format!("uv pip install requirements failed: {stderr}"),
                    }
                    .into());
                }
            }
        }

        // Install the wheel
        let output = Command::new("uv")
            .arg("pip")
            .arg("install")
            .arg("--python")
            .arg(&python_bin)
            .arg(wheel_path)
            .output()
            .await
            .map_err(|e| InstallError::PythonVenvError {
                package: package_id.name.clone(),
                message: format!("Failed to install wheel: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(InstallError::PythonVenvError {
                package: package_id.name.clone(),
                message: format!("uv pip install wheel failed: {stderr}"),
            }
            .into());
        }

        // Send wheel installed event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::PythonWheelInstalled {
                package: package_id.name.clone(),
                version: package_id.version.clone(),
            });
        }

        Ok(())
    }

    /// Create wrapper scripts for Python executables
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Script creation fails
    /// - Setting permissions fails
    pub async fn create_wrapper_scripts(
        &self,
        package_id: &PackageId,
        venv_path: &Path,
        executables: &std::collections::HashMap<String, String>,
        bin_dir: &Path,
        event_sender: Option<&EventSender>,
    ) -> Result<Vec<PathBuf>, Error> {
        let mut created_scripts = Vec::new();

        // Ensure bin directory exists
        tokio::fs::create_dir_all(bin_dir)
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "create_bin_dir".to_string(),
                path: bin_dir.display().to_string(),
                message: e.to_string(),
            })?;

        for (script_name, entry_point) in executables {
            let wrapper_path = bin_dir.join(script_name);

            // Send wrapper creating event
            if let Some(sender) = event_sender {
                let _ = sender.send(Event::PythonWrapperCreating {
                    package: package_id.name.clone(),
                    executable: script_name.clone(),
                    wrapper_path: wrapper_path.display().to_string(),
                });
            }

            // Create wrapper script content
            let wrapper_content = format!(
                r#"#!/bin/bash
# Wrapper script for {script_name} from {package_name}-{version}
# Generated by sps2

# Activate the virtual environment
source "{venv_path}/bin/activate"

# Execute the Python entry point
exec python -c "import sys; from {module} import {func}; sys.exit({func}())"
"#,
                script_name = script_name,
                package_name = package_id.name,
                version = package_id.version,
                venv_path = venv_path.display(),
                module = entry_point.split(':').next().unwrap_or(entry_point),
                func = entry_point.split(':').nth(1).unwrap_or("main"),
            );

            // Write wrapper script
            tokio::fs::write(&wrapper_path, wrapper_content)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "write_wrapper_script".to_string(),
                    path: wrapper_path.display().to_string(),
                    message: e.to_string(),
                })?;

            // Make script executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let permissions = std::fs::Permissions::from_mode(0o755);
                tokio::fs::set_permissions(&wrapper_path, permissions)
                    .await
                    .map_err(|e| InstallError::FilesystemError {
                        operation: "chmod_wrapper_script".to_string(),
                        path: wrapper_path.display().to_string(),
                        message: e.to_string(),
                    })?;
            }

            // Send wrapper created event
            if let Some(sender) = event_sender {
                let _ = sender.send(Event::PythonWrapperCreated {
                    package: package_id.name.clone(),
                    executable: script_name.clone(),
                    wrapper_path: wrapper_path.display().to_string(),
                });
            }

            created_scripts.push(wrapper_path);
        }

        Ok(created_scripts)
    }

    /// Clone a virtual environment using APFS clonefile for instant copy
    ///
    /// # Errors
    ///
    /// Returns an error if clonefile fails
    #[cfg(target_os = "macos")]
    pub async fn clone_venv(
        &self,
        package_id: &PackageId,
        source_venv: &Path,
        dest_venv: &Path,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        // Send cloning event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::PythonVenvCloning {
                package: package_id.name.clone(),
                version: package_id.version.clone(),
                from_path: source_venv.display().to_string(),
                to_path: dest_venv.display().to_string(),
            });
        }

        // Ensure parent directory exists
        if let Some(parent) = dest_venv.parent() {
            sps2_root::create_dir_all(parent).await?;
        }

        // Use APFS clonefile for instant copy
        sps2_root::clone_directory(source_venv, dest_venv).await?;

        // Send cloned event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::PythonVenvCloned {
                package: package_id.name.clone(),
                version: package_id.version.clone(),
                from_path: source_venv.display().to_string(),
                to_path: dest_venv.display().to_string(),
            });
        }

        Ok(())
    }

    /// Clone a virtual environment using regular copy (non-macOS fallback)
    #[cfg(not(target_os = "macos"))]
    pub async fn clone_venv(
        &self,
        package_id: &PackageId,
        source_venv: &Path,
        dest_venv: &Path,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        // Send cloning event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::PythonVenvCloning {
                package: package_id.name.clone(),
                version: package_id.version.clone(),
                from_path: source_venv.display().to_string(),
                to_path: dest_venv.display().to_string(),
            });
        }

        // Use regular recursive copy for non-APFS systems
        sps2_root::clone_directory(source_venv, dest_venv).await?;

        // Send cloned event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::PythonVenvCloned {
                package: package_id.name.clone(),
                version: package_id.version.clone(),
                from_path: source_venv.display().to_string(),
                to_path: dest_venv.display().to_string(),
            });
        }

        Ok(())
    }
}

/// Check if a package has Python metadata and needs venv setup
pub fn is_python_package(manifest: &Manifest) -> bool {
    manifest.python.is_some()
}
