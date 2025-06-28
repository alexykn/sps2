//! Atomic installer implementation using APFS optimizations

use crate::atomic::{rollback, transition::StateTransition};
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
        let mut transition =
            StateTransition::new(&self.state_manager, "install".to_string()).await?;

        // Set event sender on transition
        transition.event_sender.clone_from(&context.event_sender);

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::StateCreating {
                state_id: transition.staging_id,
            });
        }

        // Clone current state to staging directory
        sps2_root::create_staging_directory(&self.live_path, &transition.staging_path).await?;

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "Created staging directory at: {}",
                    transition.staging_path.display()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        // APFS clonefile already copies all existing packages, so we don't need to re-link them.
        // We only need to carry forward the package references and file information in the database.
        if let Some(parent_id) = transition.parent_id {
            let parent_packages = self
                .state_manager
                .get_installed_packages_in_state(&parent_id)
                .await?;

            for pkg in parent_packages {
                // Skip if this package is being replaced/updated
                let is_being_replaced = resolved_packages.iter().any(|(pkg_id, _)| {
                    pkg_id.name == pkg.name && pkg_id.version.to_string() == pkg.version
                });

                if !is_being_replaced {
                    // Just add the package reference - no need to re-link files
                    let package_id = sps2_resolver::PackageId::new(pkg.name.clone(), pkg.version());
                    let is_python = pkg.venv_path.is_some();

                    if !is_python {
                        let package_ref = PackageRef {
                            state_id: transition.staging_id,
                            package_id: package_id.clone(),
                            hash: pkg.hash.clone(),
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

                    // Carry forward package file information from parent state
                    // This is needed so the new state knows which files belong to this package
                    let mut tx = self.state_manager.begin_transaction().await?;
                    let file_paths = sps2_state::queries::get_package_files(
                        &mut tx,
                        &parent_id,
                        &pkg.name,
                        &pkg.version,
                    )
                    .await?;
                    tx.commit().await?;

                    // Add file paths to transition (we'll need to enhance this to include is_directory info)
                    // For now, we'll check if the path exists and is a directory
                    for file_path in file_paths {
                        let staging_file = transition.staging_path.join(&file_path);
                        let is_directory = staging_file.is_dir();
                        transition.package_files.push((
                            pkg.name.clone(),
                            pkg.version.clone(),
                            file_path,
                            is_directory,
                        ));
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

        // --- NEW 2PC COMMIT FLOW ---
        // Phase 1: Prepare and commit the database changes
        let transition_data = sps2_state::TransactionData {
            package_refs: &transition.package_refs,
            package_refs_with_venv: &transition.package_refs_with_venv,
            package_files: &transition.package_files,
            file_references: &transition.file_references,
        };
        let journal = self
            .state_manager
            .prepare_transaction(
                &transition.staging_id,
                &transition.parent_id.unwrap_or_default(),
                &transition.staging_path,
                &transition.operation,
                &transition_data,
            )
            .await?;

        // Phase 2: Execute filesystem swap and finalize
        self.state_manager
            .execute_filesystem_swap_and_finalize(journal)
            .await?;

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
                // For downloaded packages, we need to find them in the store
                // First, try to get the hash from package_map
                let (hash, store_path) = if let Some(h) = package_hash {
                    (h.clone(), self.store.package_path(h))
                } else {
                    // Try to look up from package_map first
                    if let Ok(Some(hash_hex)) = self
                        .state_manager
                        .get_package_hash(&package_id.name, &package_id.version.to_string())
                        .await
                    {
                        let hash = Hash::from_hex(&hash_hex).map_err(|e| {
                            InstallError::AtomicOperationFailed {
                                message: format!("Invalid hash in package_map: {}", e),
                            }
                        })?;
                        let store_path = self.store.package_path(&hash);
                        (hash, store_path)
                    } else {
                        // Package not in package_map yet - this means it was just downloaded
                        // We need to find it in the store and get its hash
                        // For now, we'll require the hash to be provided
                        return Err(InstallError::AtomicOperationFailed {
                            message: format!(
                                "Package {}-{} not found in package_map. This indicates a bug in the download process.",
                                package_id.name, package_id.version
                            ),
                        }.into());
                    }
                };

                // Check if this is a Python package that needs venv setup
                let stored_package = StoredPackage::load(&store_path).await?;
                let is_python = is_python_package(stored_package.manifest());

                // Get package size for store ref
                let size = stored_package.size().await?;

                // Ensure store_refs entry exists before adding to package_map
                self.state_manager
                    .ensure_store_ref(&hash.to_hex(), size as i64)
                    .await?;

                // Ensure package is in package_map for future lookups
                self.state_manager
                    .add_package_map(
                        &package_id.name,
                        &package_id.version.to_string(),
                        &hash.to_hex(),
                    )
                    .await?;

                // Link package files to staging
                self.link_package_to_staging(transition, &store_path, package_id, is_python)
                    .await?;

                // For non-Python packages, add the package reference now
                // Python packages are handled in install_python_package
                if !is_python {
                    // Calculate actual installed size
                    let size = stored_package.size().await?;

                    let package_ref = PackageRef {
                        state_id: transition.staging_id,
                        package_id: package_id.clone(),
                        hash: hash.to_hex(),
                        size: size as i64,
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
                        Some(&hash),
                    )
                    .await?;
                }
            }
            sps2_resolver::NodeAction::Local => {
                if let Some(local_path) = &node.path {
                    // Use the new staging system for local packages
                    // No hash verification needed - store handles content hashing
                    self.install_local_package_with_staging(transition, local_path, package_id)
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

        // Get the hash from the stored package
        let hash = stored_package
            .hash()
            .ok_or_else(|| InstallError::AtomicOperationFailed {
                message: "failed to get package hash from store path".to_string(),
            })?;

        // Get package size for store ref
        let size = stored_package.size().await?;

        // Ensure store_refs entry exists before adding to package_map
        self.state_manager
            .ensure_store_ref(&hash.to_hex(), size as i64)
            .await?;

        // Add to package map for future lookups
        self.state_manager
            .add_package_map(
                &package_id.name,
                &package_id.version.to_string(),
                &hash.to_hex(),
            )
            .await?;

        // Get the store path where the package was stored
        let store_path = stored_package.path();

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
        self.link_package_to_staging(transition, store_path, package_id, is_python)
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
                store_path,
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
        let staging_prefix = &transition.staging_path;
        let mut file_paths = Vec::new();

        // Load the stored package
        let stored_package = StoredPackage::load(store_path).await?;

        // Check if this is a file-level package
        if stored_package.has_file_hashes() {
            // New file-level package - link from file store
            if let Some(sender) = &transition.event_sender {
                let _ = sender.send(Event::DebugLog {
                    message: format!("Linking file-level package {} to staging", package_id.name),
                    context: std::collections::HashMap::new(),
                });
            }

            // Link files from store to staging
            stored_package.link_to(staging_prefix).await?;

            // Collect file paths for database tracking
            if let Some(file_hashes) = stored_package.file_hashes() {
                for file_hash in file_hashes {
                    file_paths.push((file_hash.relative_path.clone(), file_hash.is_directory));
                }
            }
        } else {
            // Legacy package - use existing logic
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

            // Debug log
            if let Some(sender) = &transition.event_sender {
                let _ = sender.send(Event::DebugLog {
                    message: format!(
                        "Linking package {} from {} directly to staging FHS directories",
                        package_id.name,
                        files_path.display()
                    ),
                    context: std::collections::HashMap::new(),
                });
            }

            // Recursively link files directly to their FHS locations in staging
            self.link_package_files_to_fhs(
                &files_path,
                staging_prefix,
                &files_path,
                &mut file_paths,
            )
            .await?;
        }

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
        // Paths are already relative to staging root
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

    /// Link package files directly to FHS locations without creating package directories
    async fn link_package_files_to_fhs(
        &self,
        src_dir: &Path,
        staging_root: &Path,
        base_path: &Path,
        file_paths: &mut Vec<(String, bool)>,
    ) -> Result<(), Error> {
        let mut entries = tokio::fs::read_dir(src_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let src_path = entry.path();
            let _file_name = entry.file_name();
            let metadata = entry.metadata().await?;

            // Calculate relative path from base for tracking
            let relative_from_base = src_path
                .strip_prefix(base_path)
                .unwrap_or(&src_path)
                .to_path_buf();

            // Create destination path directly in staging root
            let dest_path = staging_root.join(&relative_from_base);

            if metadata.is_dir() {
                // Create directory if it doesn't exist
                sps2_root::create_dir_all(&dest_path).await?;

                // Track directory
                let relative_path = dest_path
                    .strip_prefix(staging_root)
                    .unwrap_or(&dest_path)
                    .display()
                    .to_string();
                file_paths.push((relative_path, true));

                // Recursively process subdirectory
                Box::pin(self.link_package_files_to_fhs(
                    &src_path,
                    staging_root,
                    base_path,
                    file_paths,
                ))
                .await?;
            } else if metadata.is_file() {
                // Ensure parent directory exists
                if let Some(parent) = dest_path.parent() {
                    sps2_root::create_dir_all(parent).await?;
                }

                // Remove existing file if it exists
                if dest_path.exists() {
                    tokio::fs::remove_file(&dest_path).await?;
                }

                // Create hard link
                sps2_root::hard_link(&src_path, &dest_path).await?;

                // Track file
                let relative_path = dest_path
                    .strip_prefix(staging_root)
                    .unwrap_or(&dest_path)
                    .display()
                    .to_string();
                file_paths.push((relative_path, false));
            } else if metadata.is_symlink() {
                // For symlinks, read the target and create a new symlink
                let target = tokio::fs::read_link(&src_path).await?;

                // Ensure parent directory exists
                if let Some(parent) = dest_path.parent() {
                    sps2_root::create_dir_all(parent).await?;
                }

                // Remove existing symlink if it exists
                if dest_path.exists() {
                    tokio::fs::remove_file(&dest_path).await?;
                }

                // Create symlink
                #[cfg(unix)]
                {
                    use std::os::unix::fs::symlink;
                    symlink(&target, &dest_path)?;
                }

                // Track symlink
                let relative_path = dest_path
                    .strip_prefix(staging_root)
                    .unwrap_or(&dest_path)
                    .display()
                    .to_string();
                file_paths.push((relative_path, false));
            }
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

        // Calculate actual installed size
        let stored_package = StoredPackage::load(store_path).await?;
        let size = stored_package.size().await?;

        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
            hash: package_hash
                .ok_or_else(|| InstallError::AtomicOperationFailed {
                    message: format!(
                        "No hash available for Python package {}-{}",
                        package_id.name, package_id.version
                    ),
                })?
                .to_hex(),
            size: size as i64,
        };
        transition
            .package_refs_with_venv
            .push((package_ref, production_venv_path));

        Ok(())
    }

    /// Perform atomic uninstallation
    ///
    /// # Errors
    ///
    /// Returns an error if state transition fails, package removal fails,
    /// or filesystem operations fail.
    pub async fn uninstall(
        &mut self,
        packages_to_remove: &[PackageId],
        context: &crate::UninstallContext,
    ) -> Result<InstallResult, Error> {
        // Create new state transition
        let mut transition =
            StateTransition::new(&self.state_manager, "uninstall".to_string()).await?;

        // Set event sender on transition
        transition.event_sender.clone_from(&context.event_sender);

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::StateCreating {
                state_id: transition.staging_id,
            });
        }

        // Clone current state to staging directory
        sps2_root::create_staging_directory(&self.live_path, &transition.staging_path).await?;

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!(
                    "Created staging directory at: {}",
                    transition.staging_path.display()
                ),
                context: std::collections::HashMap::new(),
            });
        }

        // APFS clonefile already copies all existing packages, so we don't need to re-link them.
        // We only need to remove the packages being uninstalled and carry forward other package references.
        let mut result = InstallResult::new(transition.staging_id);

        if let Some(parent_id) = transition.parent_id {
            let parent_packages = self
                .state_manager
                .get_installed_packages_in_state(&parent_id)
                .await?;

            for pkg in parent_packages {
                // Check if this package should be removed
                let should_remove = packages_to_remove
                    .iter()
                    .any(|remove_pkg| remove_pkg.name == pkg.name);

                if should_remove {
                    // Remove package files from staging
                    result.add_removed(PackageId::new(pkg.name.clone(), pkg.version()));

                    // Remove venv if it exists
                    if let Some(venv_path) = &pkg.venv_path {
                        self.remove_package_venv(&pkg, venv_path, context).await?;
                    }

                    // Remove package files from staging
                    self.remove_package_from_staging(&mut transition, &pkg)
                        .await?;

                    if let Some(sender) = &context.event_sender {
                        let _ = sender.send(Event::DebugLog {
                            message: format!("Removed package {} from staging", pkg.name),
                            context: std::collections::HashMap::new(),
                        });
                    }
                } else {
                    // Keep this package - just add the reference, no need to re-link files
                    let package_id = PackageId::new(pkg.name.clone(), pkg.version());
                    let is_python = pkg.venv_path.is_some();

                    if !is_python {
                        let package_ref = PackageRef {
                            state_id: transition.staging_id,
                            package_id: package_id.clone(),
                            hash: pkg.hash.clone(),
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

                    // Carry forward package file information from parent state
                    let mut tx = self.state_manager.begin_transaction().await?;
                    let file_paths = sps2_state::queries::get_package_files(
                        &mut tx,
                        &parent_id,
                        &pkg.name,
                        &pkg.version,
                    )
                    .await?;
                    tx.commit().await?;

                    // Add file paths to transition
                    for file_path in file_paths {
                        let staging_file = transition.staging_path.join(&file_path);
                        let is_directory = staging_file.is_dir();
                        transition.package_files.push((
                            pkg.name.clone(),
                            pkg.version.clone(),
                            file_path,
                            is_directory,
                        ));
                    }
                }
            }
        }

        if context.dry_run {
            // Clean up staging and return result without committing
            transition.cleanup().await?;
            return Ok(result);
        }

        // --- NEW 2PC COMMIT FLOW ---
        // Phase 1: Prepare and commit the database changes
        let transition_data = sps2_state::TransactionData {
            package_refs: &transition.package_refs,
            package_refs_with_venv: &transition.package_refs_with_venv,
            package_files: &transition.package_files,
            file_references: &transition.file_references,
        };
        let journal = self
            .state_manager
            .prepare_transaction(
                &transition.staging_id,
                &transition.parent_id.unwrap_or_default(),
                &transition.staging_path,
                &transition.operation,
                &transition_data,
            )
            .await?;

        // Phase 2: Execute filesystem swap and finalize
        self.state_manager
            .execute_filesystem_swap_and_finalize(journal)
            .await?;

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::StateTransition {
                from: transition.parent_id.unwrap_or_default(),
                to: transition.staging_id,
                operation: "uninstall".to_string(),
            });
        }

        Ok(result)
    }

    /// Remove package files from staging directory
    async fn remove_package_from_staging(
        &self,
        transition: &mut StateTransition,
        package: &sps2_state::models::Package,
    ) -> Result<(), Error> {
        // Get all files belonging to this package from the database
        let mut tx = self.state_manager.begin_transaction().await?;
        let file_paths =
            sps2_state::queries::get_active_package_files(&mut tx, &package.name, &package.version)
                .await?;
        tx.commit().await?;

        // Group files by type for proper removal order
        let mut symlinks = Vec::new();
        let mut regular_files = Vec::new();
        let mut directories = Vec::new();

        for file_path in file_paths {
            let staging_file = transition.staging_path.join(&file_path);

            if staging_file.exists() {
                // Check if it's a symlink
                let metadata = tokio::fs::symlink_metadata(&staging_file).await?;
                if metadata.is_symlink() {
                    symlinks.push(file_path);
                } else if staging_file.is_dir() {
                    directories.push(file_path);
                } else {
                    regular_files.push(file_path);
                }
            }
        }

        // Remove in order: symlinks first, then files, then directories
        // This ensures we don't try to remove non-empty directories

        // 1. Remove symlinks
        for file_path in symlinks {
            let staging_file = transition.staging_path.join(&file_path);
            if staging_file.exists() {
                tokio::fs::remove_file(&staging_file).await.map_err(|e| {
                    InstallError::FilesystemError {
                        operation: "remove_symlink".to_string(),
                        path: staging_file.display().to_string(),
                        message: e.to_string(),
                    }
                })?;
            }
        }

        // 2. Remove regular files
        for file_path in regular_files {
            let staging_file = transition.staging_path.join(&file_path);
            if staging_file.exists() {
                tokio::fs::remove_file(&staging_file).await.map_err(|e| {
                    InstallError::FilesystemError {
                        operation: "remove_file".to_string(),
                        path: staging_file.display().to_string(),
                        message: e.to_string(),
                    }
                })?;
            }
        }

        // 3. Remove directories in reverse order (deepest first)
        directories.sort_by(|a, b| b.cmp(a)); // Reverse lexicographic order
        for file_path in directories {
            let staging_file = transition.staging_path.join(&file_path);
            if staging_file.exists() {
                // Try to remove directory if it's empty
                if let Ok(mut entries) = tokio::fs::read_dir(&staging_file).await {
                    if entries.next_entry().await?.is_none() {
                        tokio::fs::remove_dir(&staging_file).await.map_err(|e| {
                            InstallError::FilesystemError {
                                operation: "remove_dir".to_string(),
                                path: staging_file.display().to_string(),
                                message: e.to_string(),
                            }
                        })?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Remove Python virtual environment for a package
    async fn remove_package_venv(
        &self,
        package: &sps2_state::models::Package,
        venv_path: &str,
        context: &crate::UninstallContext,
    ) -> Result<(), Error> {
        let venv_path_buf = std::path::Path::new(venv_path);

        // Send event about venv removal
        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::PythonVenvRemoving {
                package: package.name.clone(),
                version: sps2_types::Version::parse(&package.version)
                    .unwrap_or_else(|_| sps2_types::Version::new(0, 0, 0)),
                venv_path: venv_path_buf.display().to_string(),
            });
        }

        // Remove the venv directory if it exists
        if venv_path_buf.exists() {
            sps2_root::remove_dir_all(venv_path_buf).await?;

            if let Some(sender) = &context.event_sender {
                let _ = sender.send(Event::PythonVenvRemoved {
                    package: package.name.clone(),
                    version: sps2_types::Version::parse(&package.version)
                        .unwrap_or_else(|_| sps2_types::Version::new(0, 0, 0)),
                    venv_path: venv_path_buf.display().to_string(),
                });
            }
        }

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
