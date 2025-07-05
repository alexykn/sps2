//! Atomic installer implementation using APFS optimizations

use crate::atomic::{rollback, transition::StateTransition};
// Removed Python venv handling - Python packages are now handled like regular packages
use crate::{InstallContext, InstallResult, StagingManager};
use sps2_errors::{Error, InstallError};
use sps2_events::Event;
use sps2_hash::Hash;
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

        Ok(Self {
            state_manager,
            store,
            staging_manager,
            live_path,
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

                    // Add package reference (Python packages are now treated like regular packages)
                    let package_ref = PackageRef {
                        state_id: transition.staging_id,
                        package_id: package_id.clone(),
                        hash: pkg.hash.clone(),
                        size: pkg.size,
                    };
                    transition.package_refs.push(package_ref);

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
            package_files: &transition.package_files,
            file_references: &transition.file_references,
            pending_file_hashes: &transition.pending_file_hashes,
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

                // Load package for size calculation
                let stored_package = StoredPackage::load(&store_path).await?;

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
                self.link_package_to_staging(transition, &store_path, package_id)
                    .await?;

                // Add the package reference
                let package_ref = PackageRef {
                    state_id: transition.staging_id,
                    package_id: package_id.clone(),
                    hash: hash.to_hex(),
                    size: size as i64,
                };
                transition.package_refs.push(package_ref);
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

        // Link package files from store to staging
        self.link_package_to_staging(transition, store_path, package_id)
            .await?;

        // Add the package reference
        let size = stored_package.size().await?;
        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
            hash: hash.to_hex(),
            size: size as i64,
        };
        transition.package_refs.push(package_ref);

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
    ) -> Result<(), Error> {
        let staging_prefix = &transition.staging_path;
        let mut file_paths = Vec::new();

        // Load the stored package
        let stored_package = StoredPackage::load(store_path).await?;

        // Link files from store to staging
        if let Some(sender) = &transition.event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!("Linking package {} to staging", package_id.name),
                context: std::collections::HashMap::new(),
            });
        }

        stored_package.link_to(staging_prefix).await?;

        // Collect file paths for database tracking AND store file hash info
        if let Some(file_hashes) = stored_package.file_hashes() {
            // Store the file hash information for later use when we have package IDs
            transition
                .pending_file_hashes
                .push((package_id.clone(), file_hashes.to_vec()));

            for file_hash in file_hashes {
                // Skip manifest.toml and sbom files - they should not be tracked in package_files
                if !file_hash.is_directory
                    && (file_hash.relative_path == "manifest.toml"
                        || file_hash.relative_path == "sbom.spdx.json"
                        || file_hash.relative_path == "sbom.cdx.json")
                {
                    continue;
                }

                file_paths.push((file_hash.relative_path.clone(), file_hash.is_directory));
            }
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

    // Removed install_python_package - Python packages are now handled like regular packages

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

                    // No special cleanup needed - all packages are handled the same way

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
                    let package_ref = PackageRef {
                        state_id: transition.staging_id,
                        package_id: package_id.clone(),
                        hash: pkg.hash.clone(),
                        size: pkg.size,
                    };
                    transition.package_refs.push(package_ref);

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
            package_files: &transition.package_files,
            file_references: &transition.file_references,
            pending_file_hashes: &transition.pending_file_hashes,
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

    // Removed remove_package_venv - Python packages are now handled like regular packages

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
