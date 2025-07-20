//! Atomic installer implementation using APFS optimizations

use crate::atomic::{rollback, transition::StateTransition};
use std::sync::Arc;
// Removed Python venv handling - Python packages are now handled like regular packages
use crate::{InstallContext, InstallResult, PreparedPackage, StagingManager};
use sps2_errors::{Error, InstallError};
use sps2_events::{Event, EventEmitter, EventSender};

use sps2_resolver::{PackageId, ResolvedNode};
use sps2_state::{PackageRef, StateManager};
use sps2_store::{PackageStore, StoredPackage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Implement EventEmitter for InstallContext
impl EventEmitter for InstallContext {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

/// Implement EventEmitter for UninstallContext
impl EventEmitter for crate::UninstallContext {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

/// Atomic installer using APFS optimizations
pub struct AtomicInstaller {
    /// State manager for atomic transitions
    state_manager: StateManager,
    /// Live prefix path
    live_path: PathBuf,
}

impl AtomicInstaller {
    /// Execute two-phase commit flow for a transition
    async fn execute_two_phase_commit<T: EventEmitter>(
        &self,
        transition: &StateTransition,
        context: &T,
    ) -> Result<(), Error> {
        let parent_id = transition.parent_id.unwrap_or_default();

        // Emit 2PC start event
        context.emit_event(Event::TwoPhaseCommitStarting {
            state_id: transition.staging_id,
            parent_state_id: parent_id,
            operation: transition.operation.clone(),
        });

        // Phase 1: Prepare and commit the database changes
        context.emit_event(Event::TwoPhaseCommitPhaseOneStarting {
            state_id: transition.staging_id,
            operation: transition.operation.clone(),
        });

        let transition_data = sps2_state::TransactionData {
            package_refs: &transition.package_refs,
            package_files: &transition.package_files,
            file_references: &transition.file_references,
            pending_file_hashes: &transition.pending_file_hashes,
        };

        let journal = match self
            .state_manager
            .prepare_transaction(
                &transition.staging_id,
                &parent_id,
                &transition.staging_path,
                &transition.operation,
                &transition_data,
            )
            .await
        {
            Ok(journal) => {
                context.emit_event(Event::TwoPhaseCommitPhaseOneCompleted {
                    state_id: transition.staging_id,
                    operation: transition.operation.clone(),
                });
                journal
            }
            Err(e) => {
                context.emit_event(Event::TwoPhaseCommitFailed {
                    state_id: transition.staging_id,
                    operation: transition.operation.clone(),
                    error: e.to_string(),
                    phase: "phase_one".to_string(),
                });
                return Err(e);
            }
        };

        // Phase 2: Execute filesystem swap and finalize
        context.emit_event(Event::TwoPhaseCommitPhaseTwoStarting {
            state_id: transition.staging_id,
            operation: transition.operation.clone(),
        });

        match self
            .state_manager
            .execute_filesystem_swap_and_finalize(journal)
            .await
        {
            Ok(()) => {
                context.emit_event(Event::TwoPhaseCommitPhaseTwoCompleted {
                    state_id: transition.staging_id,
                    operation: transition.operation.clone(),
                });
            }
            Err(e) => {
                context.emit_event(Event::TwoPhaseCommitFailed {
                    state_id: transition.staging_id,
                    operation: transition.operation.clone(),
                    error: e.to_string(),
                    phase: "phase_two".to_string(),
                });
                return Err(e);
            }
        }

        // Emit 2PC completion event
        context.emit_event(Event::TwoPhaseCommitCompleted {
            state_id: transition.staging_id,
            parent_state_id: parent_id,
            operation: transition.operation.clone(),
        });

        Ok(())
    }

    /// Carry forward packages from parent state, excluding specified packages
    async fn carry_forward_packages(
        &self,
        transition: &mut StateTransition,
        exclude_packages: &[PackageId],
    ) -> Result<(), Error> {
        if let Some(parent_id) = transition.parent_id {
            let parent_packages = self
                .state_manager
                .get_installed_packages_in_state(&parent_id)
                .await?;

            for pkg in parent_packages {
                // Check if this package should be excluded
                let should_exclude = exclude_packages.iter().any(|exclude_pkg| {
                    exclude_pkg.name == pkg.name && exclude_pkg.version.to_string() == pkg.version
                });

                if !should_exclude {
                    // Just add the package reference - no need to re-link files
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
        Ok(())
    }

    /// Setup state transition and staging directory
    async fn setup_state_transition<T: EventEmitter>(
        &self,
        operation: &str,
        context: &T,
    ) -> Result<StateTransition, Error> {
        // Create new state transition
        let mut transition =
            StateTransition::new(&self.state_manager, operation.to_string()).await?;

        // Set event sender on transition
        transition.event_sender = context.event_sender().cloned();

        context.emit_event(Event::StateCreating {
            state_id: transition.staging_id,
        });

        // Clone current state to staging directory
        sps2_root::create_staging_directory(&self.live_path, &transition.staging_path).await?;

        context.emit_debug(format!(
            "Created staging directory at: {}",
            transition.staging_path.display()
        ));

        Ok(transition)
    }

    /// Create new atomic installer
    ///
    /// # Errors
    ///
    /// Returns an error if staging manager initialization fails
    pub async fn new(state_manager: StateManager, store: PackageStore) -> Result<Self, Error> {
        // Derive staging base path from StateManager's state path for test isolation
        let staging_base_path = state_manager.state_path().join("staging");
        let resources = Arc::new(sps2_resources::ResourceManager::default());
        let _staging_manager =
            StagingManager::new(store.clone(), staging_base_path, resources).await?;
        let live_path = state_manager.live_path().to_path_buf();

        Ok(Self {
            state_manager,
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
        prepared_packages: Option<&HashMap<PackageId, PreparedPackage>>,
    ) -> Result<InstallResult, Error> {
        // Setup state transition and staging directory
        let mut transition = self.setup_state_transition("install", context).await?;

        // APFS clonefile already copies all existing packages, so we don't need to re-link them.
        // We only need to carry forward the package references and file information in the database.
        let exclude_packages: Vec<PackageId> = resolved_packages.keys().cloned().collect();
        self.carry_forward_packages(&mut transition, &exclude_packages)
            .await?;

        // Apply package changes to staging
        let mut result = InstallResult::new(transition.staging_id);

        for (package_id, node) in resolved_packages {
            let prepared_package = prepared_packages.and_then(|packages| packages.get(package_id));
            self.install_package_to_staging(
                &mut transition,
                package_id,
                node,
                prepared_package,
                &mut result,
            )
            .await?;
        }

        if context.dry_run {
            // Clean up staging and return result without committing
            transition.cleanup(&self.state_manager).await?;
            return Ok(result);
        }

        // Execute two-phase commit
        self.execute_two_phase_commit(&transition, context).await?;

        context.emit_event(Event::StateTransition {
            from: transition.parent_id.unwrap_or_default(),
            to: transition.staging_id,
            operation: "install".to_string(),
        });

        Ok(result)
    }

    /// Install a single package to staging directory
    async fn install_package_to_staging(
        &self,
        transition: &mut StateTransition,
        package_id: &PackageId,
        node: &ResolvedNode,
        prepared_package: Option<&PreparedPackage>,
        result: &mut InstallResult,
    ) -> Result<(), Error> {
        // Install the package files (both Download and Local actions are handled identically)
        let action_name = match &node.action {
            sps2_resolver::NodeAction::Download => "downloaded",
            sps2_resolver::NodeAction::Local => "local",
        };

        let prepared = prepared_package.ok_or_else(|| {
            InstallError::AtomicOperationFailed {
                message: format!(
                    "Missing prepared package data for {} package {}-{}. This indicates a bug in ParallelExecutor.",
                    action_name, package_id.name, package_id.version
                ),
            }
        })?;

        let hash = &prepared.hash;
        let store_path = &prepared.store_path;
        let size = prepared.size;

        // Load package from the prepared store path
        let _stored_package = StoredPackage::load(store_path).await?;

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
        self.link_package_to_staging(transition, store_path, package_id)
            .await?;

        // Add the package reference
        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
            hash: hash.to_hex(),
            size: size as i64,
        };
        transition.package_refs.push(package_ref);

        result.add_installed(package_id.clone());
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
        // Setup state transition and staging directory
        let mut transition = self.setup_state_transition("uninstall", context).await?;

        // APFS clonefile already copies all existing packages, so we don't need to re-link them.
        // We only need to remove the packages being uninstalled and carry forward other package references.
        let mut result = InstallResult::new(transition.staging_id);

        // Remove packages from staging and track them in result
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

                    // Remove package files from staging
                    self.remove_package_from_staging(&mut transition, &pkg)
                        .await?;

                    context.emit_debug(format!("Removed package {} from staging", pkg.name));
                }
            }
        }

        // Carry forward packages that are not being removed
        self.carry_forward_packages(&mut transition, packages_to_remove)
            .await?;

        if context.dry_run {
            // Clean up staging and return result without committing
            transition.cleanup(&self.state_manager).await?;
            return Ok(result);
        }

        // Execute two-phase commit
        self.execute_two_phase_commit(&transition, context).await?;

        context.emit_event(Event::StateTransition {
            from: transition.parent_id.unwrap_or_default(),
            to: transition.staging_id,
            operation: "uninstall".to_string(),
        });

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
