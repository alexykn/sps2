//! Atomic installer implementation using APFS optimizations

use crate::atomic::{linking, rollback, transition::StateTransition};
use crate::python::{is_python_package, PythonVenvManager};
use crate::{InstallContext, InstallResult, StagingManager};
use sps2_errors::{Error, InstallError};
use sps2_events::Event;
use sps2_hash::Hash;
use sps2_manifest::Manifest;
use sps2_resolver::{PackageId, ResolvedNode};
use sps2_state::{PackageRef, StateManager};
use sps2_store::{PackageStore, StoredPackage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Atomic installer using APFS optimizations
pub struct AtomicInstaller {
    /// State manager for atomic transitions
    state_manager: StateManager,
    /// Package store
    store: PackageStore,
    /// Staging manager for secure extraction
    staging_manager: StagingManager,
    /// Live prefix path
    live_path: PathBuf,
    /// Python virtual environment manager
    python_venv_manager: PythonVenvManager,
}

impl AtomicInstaller {
    /// Create new atomic installer
    ///
    /// # Errors
    ///
    /// Returns an error if staging manager initialization fails
    pub async fn new(state_manager: StateManager, store: PackageStore) -> Result<Self, Error> {
        // Derive staging base path from StateManager's state path for test isolation
        let staging_base_path = state_manager.state_path().join("staging");
        let staging_manager = StagingManager::new(store.clone(), staging_base_path).await?;
        let live_path = state_manager.live_path().to_path_buf();

        // Initialize Python venv manager with base path
        let venvs_base = PathBuf::from("/opt/pm/venvs");
        let python_venv_manager = PythonVenvManager::new(venvs_base);

        Ok(Self {
            state_manager,
            store,
            staging_manager,
            live_path,
            python_venv_manager,
        })
    }

    /// Perform atomic installation
    ///
    /// # Errors
    ///
    /// Returns an error if state transition fails, package installation fails,
    /// or filesystem operations fail.
    pub async fn install(
        &mut self,
        context: &InstallContext,
        resolved_packages: &HashMap<PackageId, ResolvedNode>,
        package_hashes: Option<&HashMap<PackageId, Hash>>,
    ) -> Result<InstallResult, Error> {
        // Create new state transition
        let mut transition = StateTransition::new(&self.state_manager).await?;

        // Set event sender on transition
        transition.event_sender.clone_from(&context.event_sender);

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::StateCreating {
                state_id: transition.staging_id,
            });
        }

        // Clone current state to staging directory
        transition.create_staging(&self.live_path)?;

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "Created staging directory at: {}",
                    transition.staging_path.display()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        // Get packages from the parent state to ensure they are carried over
        if let Some(parent_id) = transition.parent_id {
            let parent_packages = self
                .state_manager
                .get_installed_packages_in_state(&parent_id)
                .await?;

            for pkg in parent_packages {
                // Re-link the files for each old package into the new staging directory
                // Skip if this package is being replaced/updated
                let is_being_replaced = resolved_packages.iter().any(|(pkg_id, _)| {
                    pkg_id.name == pkg.name && pkg_id.version.to_string() == pkg.version
                });

                if !is_being_replaced {
                    // Get the store path using the hash
                    let store_path = if pkg.hash == "placeholder-hash" {
                        // Use the temporary path based on name/version
                        self.store.get_package_path(&pkg.name, &pkg.version())?
                    } else {
                        let hash = Hash::from_hex(&pkg.hash).map_err(|e| {
                            InstallError::AtomicOperationFailed {
                                message: format!(
                                    "Invalid hash in database for {}: {}",
                                    pkg.name, e
                                ),
                            }
                        })?;
                        self.store.package_path(&hash)
                    };

                    if store_path.exists() {
                        let package_id =
                            sps2_resolver::PackageId::new(pkg.name.clone(), pkg.version());

                        // Check if it's a Python package
                        let is_python = pkg.venv_path.is_some();

                        if let Some(sender) = &context.event_sender {
                            let _ = sender.send(Event::DebugLog {
                                message: format!(
                                    "Re-linking existing package {} from parent state",
                                    pkg.name
                                ),
                                context: std::collections::HashMap::new(),
                            });
                        }

                        self.link_package_to_staging(
                            &mut transition,
                            &store_path,
                            &package_id,
                            is_python,
                        )
                        .await?;

                        // Add the existing package to transition.package_refs so it's recorded in the new state
                        if !is_python {
                            let package_ref = PackageRef {
                                state_id: transition.staging_id,
                                package_id: package_id.clone(),
                                hash: pkg.hash.clone(), // Use the hash from the database
                                size: pkg.size,
                            };
                            transition.package_refs.push(package_ref);
                        } else if let Some(venv_path) = &pkg.venv_path {
                            // For Python packages, add with venv path
                            let package_ref = PackageRef {
                                state_id: transition.staging_id,
                                package_id: package_id.clone(),
                                hash: pkg.hash.clone(),
                                size: pkg.size,
                            };
                            transition
                                .package_refs_with_venv
                                .push((package_ref, venv_path.clone()));
                        }
                    }
                }
            }
        }

        // Apply package changes to staging
        let mut result = InstallResult::new(transition.staging_id);

        for (package_id, node) in resolved_packages {
            let package_hash = package_hashes.and_then(|hashes| hashes.get(package_id));
            self.install_package_to_staging(
                &mut transition,
                package_id,
                node,
                package_hash,
                &mut result,
            )
            .await?;
        }

        if context.dry_run {
            // Clean up staging and return result without committing
            transition.cleanup().await?;
            return Ok(result);
        }

        // Commit the state transition
        transition.commit(&self.live_path).await?;

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::StateTransition {
                from: transition.parent_id.unwrap_or_default(),
                to: transition.staging_id,
                operation: "install".to_string(),
            });
        }

        Ok(result)
    }

    /// Install a single package to staging directory
    async fn install_package_to_staging(
        &self,
        transition: &mut StateTransition,
        package_id: &PackageId,
        node: &ResolvedNode,
        package_hash: Option<&Hash>,
        result: &mut InstallResult,
    ) -> Result<(), Error> {
        // First, install the package files
        match &node.action {
            sps2_resolver::NodeAction::Download => {
                // Get the store path using the hash if available, otherwise fall back to name/version lookup
                let store_path = if let Some(hash) = package_hash {
                    self.store.package_path(hash)
                } else {
                    // Fallback for when hash is not available (shouldn't happen in normal flow)
                    self.store
                        .get_package_path(&package_id.name, &package_id.version)?
                };

                // Check if this is a Python package that needs venv setup
                let stored_package = StoredPackage::load(&store_path).await?;
                let is_python = is_python_package(stored_package.manifest());

                // Link package files to staging
                self.link_package_to_staging(transition, &store_path, package_id, is_python)
                    .await?;

                // For non-Python packages, add the package reference now
                // Python packages are handled in install_python_package
                if !is_python {
                    let package_ref = PackageRef {
                        state_id: transition.staging_id,
                        package_id: package_id.clone(),
                        hash: package_hash.map_or_else(
                            || "placeholder-hash".to_string(),
                            sps2_hash::Hash::to_hex,
                        ),
                        // TODO: Calculate actual installed size from manifest or store
                        size: 0,
                    };
                    transition.package_refs.push(package_ref);
                }

                // If it's a Python package, also set up the venv
                if is_python {
                    self.install_python_package(
                        transition,
                        package_id,
                        stored_package.manifest(),
                        &store_path,
                        package_hash,
                    )
                    .await?;
                }
            }
            sps2_resolver::NodeAction::Local => {
                if let Some(local_path) = &node.path {
                    // For local packages, compute the hash if not provided
                    let computed_hash;
                    let hash_to_use = if let Some(hash) = package_hash {
                        hash
                    } else {
                        // Compute hash if not provided
                        computed_hash = Hash::hash_file(local_path).await?;
                        &computed_hash
                    };

                    // Use the new staging system for local packages
                    self.install_local_package_with_staging(
                        transition,
                        local_path,
                        package_id,
                        Some(hash_to_use),
                    )
                    .await?;
                }
            }
        }

        result.add_installed(package_id.clone());
        Ok(())
    }

    /// Install a local package using the staging system
    async fn install_local_package_with_staging(
        &self,
        transition: &mut StateTransition,
        local_path: &Path,
        package_id: &PackageId,
        package_hash: Option<&Hash>,
    ) -> Result<(), Error> {
        // Extract to staging directory with validation
        let staging_dir = self
            .staging_manager
            .extract_to_staging(local_path, package_id, None)
            .await?;

        // Create staging guard for automatic cleanup on failure
        let mut staging_guard = crate::StagingGuard::new(staging_dir);

        // Get the validated staging directory
        let _staging_dir =
            staging_guard
                .staging_dir()
                .ok_or_else(|| InstallError::AtomicOperationFailed {
                    message: "staging directory unavailable".to_string(),
                })?;

        // Add package to store from staging directory
        let stored_package = self.store.add_package(local_path).await?;

        // Use provided hash or compute it
        let hash = if let Some(h) = package_hash {
            h.clone()
        } else {
            Hash::hash_file(local_path).await?
        };

        // Get the store path where the package was stored
        let store_path = self.store.package_path(&hash);

        // Check if this is a Python package
        let is_python = is_python_package(stored_package.manifest());

        // Debug log
        if let Some(sender) = &transition.event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "Linking local package {} from store {} to staging",
                    package_id.name,
                    store_path.display()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        // Link package files from store to staging (same as downloaded packages)
        self.link_package_to_staging(transition, &store_path, package_id, is_python)
            .await?;

        // For non-Python packages, add the package reference now
        if !is_python {
            let size = stored_package.size().await?;
            let package_ref = PackageRef {
                state_id: transition.staging_id,
                package_id: package_id.clone(),
                hash: hash.to_hex(),
                size: size as i64,
            };
            transition.package_refs.push(package_ref);
        } else {
            // If it's a Python package, set up the venv
            self.install_python_package(
                transition,
                package_id,
                stored_package.manifest(),
                &store_path,
                Some(&hash),
            )
            .await?;
        }

        // Successfully processed - prevent cleanup
        let _staging_dir = staging_guard.take()?;

        Ok(())
    }

    /// Link package from store to staging directory
    async fn link_package_to_staging(
        &self,
        transition: &mut StateTransition,
        store_path: &Path,
        package_id: &PackageId,
        _is_python: bool,
    ) -> Result<(), Error> {
        // Create hard links from store to staging directory and collect file paths
        let staging_prefix = &transition.staging_path;
        let mut file_paths = Vec::new();

        // Load the stored package to get the correct files path
        let stored_package = StoredPackage::load(store_path).await?;
        let files_path = stored_package.files_path();

        // Check if files directory exists
        if !files_path.exists() {
            return Err(InstallError::AtomicOperationFailed {
                message: format!(
                    "Package files directory not found at {}",
                    files_path.display()
                ),
            }
            .into());
        }

        // The files_path points to the package directory (e.g., hello-cargo-1.0.0/)
        // We need to link its contents to staging while preserving directory structure

        // Debug log
        if let Some(sender) = &transition.event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "Linking package {} from {} to staging at {}",
                    package_id.name,
                    files_path.display(),
                    staging_prefix.display()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        // Recursively link the entire package directory contents to staging
        linking::create_hardlinks_recursive_with_tracking(
            &files_path,
            staging_prefix,
            &files_path,
            &mut file_paths,
        )
        .await?;

        // Debug what was linked
        if let Some(sender) = &transition.event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "Linked {} files/directories for package {}",
                    file_paths.len(),
                    package_id.name
                ),
                context: std::collections::HashMap::new(),
            });
        }

        // Store file information to be added during commit
        for (file_path, is_directory) in file_paths {
            transition.package_files.push((
                package_id.name.clone(),
                package_id.version.to_string(),
                file_path,
                is_directory,
            ));
        }

        Ok(())
    }

    /// Install Python package with virtual environment
    async fn install_python_package(
        &self,
        transition: &mut StateTransition,
        package_id: &PackageId,
        manifest: &Manifest,
        store_path: &Path,
        package_hash: Option<&Hash>,
    ) -> Result<(), Error> {
        let python_metadata = manifest
            .python
            .as_ref()
            .ok_or_else(|| InstallError::Failed {
                message: format!("Package {} has no Python metadata", package_id.name),
            })?;

        let event_sender = transition.event_sender.as_ref();

        // Create virtual environment in the venvs directory
        let venv_path = self
            .python_venv_manager
            .create_venv(package_id, python_metadata, event_sender)
            .await?;

        // Get paths to wheel and requirements files
        let stored_package = StoredPackage::load(store_path).await?;
        let files_path = stored_package.files_path();
        let wheel_path = files_path.join(&python_metadata.wheel_file);
        let requirements_path = if python_metadata.requirements_file.is_empty() {
            None
        } else {
            Some(files_path.join(&python_metadata.requirements_file))
        };

        // Install wheel into venv
        self.python_venv_manager
            .install_wheel(
                package_id,
                &venv_path,
                &wheel_path,
                requirements_path.as_deref(),
                event_sender,
            )
            .await?;

        // Create wrapper scripts in staging bin directory
        let staging_bin_dir = transition.staging_path.join("bin");
        let created_scripts = self
            .python_venv_manager
            .create_wrapper_scripts(
                package_id,
                &venv_path,
                &python_metadata.executables,
                &staging_bin_dir,
                event_sender,
            )
            .await?;

        // Track the wrapper scripts as package files
        for script_path in created_scripts {
            let relative_path = script_path
                .strip_prefix(&transition.staging_path)
                .unwrap_or(&script_path);
            transition.package_files.push((
                package_id.name.clone(),
                package_id.version.to_string(),
                relative_path.display().to_string(),
                false, // scripts are files, not directories
            ));
        }

        // Clone venv to staging for atomic installation
        let staging_venv_path = transition
            .staging_path
            .parent()
            .unwrap_or(&transition.staging_path)
            .join("venvs")
            .join(format!("{}-{}", package_id.name, package_id.version));

        self.python_venv_manager
            .clone_venv(package_id, &venv_path, &staging_venv_path, event_sender)
            .await?;

        // Store package reference with venv path to be added during commit
        // Get the actual venv path that will be used in production
        let production_venv_path =
            format!("/opt/pm/venvs/{}-{}", package_id.name, package_id.version);

        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
            hash: package_hash
                .map_or_else(|| "placeholder-hash".to_string(), sps2_hash::Hash::to_hex),
            // TODO: Calculate actual installed size for python packages
            size: 0,
        };
        transition
            .package_refs_with_venv
            .push((package_ref, production_venv_path));

        Ok(())
    }

    /// Rollback to a previous state
    ///
    /// # Errors
    ///
    /// Returns an error if the target state doesn't exist, filesystem swap fails,
    /// or database update fails.
    pub async fn rollback(&mut self, target_state_id: Uuid) -> Result<(), Error> {
        rollback::rollback_to_state(&self.state_manager, &self.live_path, target_state_id).await
    }
}
