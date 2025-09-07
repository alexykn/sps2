//! Atomic installer implementation using APFS optimizations

use crate::atomic::transition::StateTransition;
use std::sync::Arc;
// Removed Python venv handling - Python packages are now handled like regular packages
use crate::{InstallContext, InstallResult, PreparedPackage, StagingManager};
use sps2_errors::{Error, InstallError};
use sps2_events::{AppEvent, EventEmitter, EventSender, GeneralEvent, StateEvent};
use sps2_platform::{core::PlatformContext, PlatformManager};

use sps2_resolver::{PackageId, ResolvedNode};
use sps2_state::{PackageRef, StateManager};
use sps2_store::{PackageStore, StoredPackage};
use std::collections::{HashMap, HashSet};
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

/// Implement EventEmitter for UpdateContext
impl EventEmitter for crate::UpdateContext {
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
    /// Content-addressable package store
    store: PackageStore,
}

impl AtomicInstaller {
    /// Create a platform context for filesystem operations
    fn create_platform_context(&self) -> (&'static sps2_platform::Platform, PlatformContext) {
        let platform = PlatformManager::instance().platform();
        let context = platform.create_context(None);
        (platform, context)
    }

    /// Create staging directory using platform abstraction
    async fn create_staging_directory(
        &self,
        live_path: &Path,
        staging_path: &Path,
    ) -> Result<(), Error> {
        let (platform, ctx) = self.create_platform_context();

        if platform.filesystem().exists(&ctx, live_path).await {
            // Live directory exists - clone it to staging

            // Ensure parent directory exists for staging path
            if let Some(parent) = staging_path.parent() {
                platform
                    .filesystem()
                    .create_dir_all(&ctx, parent)
                    .await
                    .map_err(|e| InstallError::FilesystemError {
                        operation: "create_dir_all".to_string(),
                        path: parent.display().to_string(),
                        message: e.to_string(),
                    })?;
            }

            // Remove existing staging directory if it exists (required for APFS clonefile)
            if platform.filesystem().exists(&ctx, staging_path).await {
                platform
                    .filesystem()
                    .remove_dir_all(&ctx, staging_path)
                    .await
                    .map_err(|e| InstallError::FilesystemError {
                        operation: "remove_dir_all".to_string(),
                        path: staging_path.display().to_string(),
                        message: e.to_string(),
                    })?;
            }

            // Clone the live directory to staging using APFS clonefile
            platform
                .filesystem()
                .clone_directory(&ctx, live_path, staging_path)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "clone_directory".to_string(),
                    path: format!("{} -> {}", live_path.display(), staging_path.display()),
                    message: e.to_string(),
                })?;
        } else {
            // No live directory exists - create empty staging directory for fresh installation
            platform
                .filesystem()
                .create_dir_all(&ctx, staging_path)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "create_dir_all".to_string(),
                    path: staging_path.display().to_string(),
                    message: e.to_string(),
                })?;
        }

        Ok(())
    }
    /// Execute two-phase commit flow for a transition
    async fn execute_two_phase_commit<T: EventEmitter>(
        &self,
        transition: &StateTransition,
        context: &T,
    ) -> Result<(), Error> {
        let parent_id = transition.parent_id.unwrap_or_default();

        // Emit 2PC start event
        context.emit(AppEvent::State(StateEvent::TwoPhaseCommitStarting {
            state_id: transition.staging_id,
            parent_state_id: parent_id,
            operation: transition.operation.clone(),
        }));

        // Phase 1: Prepare and commit the database changes
        context.emit(AppEvent::State(
            StateEvent::TwoPhaseCommitPhaseOneStarting {
                state_id: transition.staging_id,
                operation: transition.operation.clone(),
            },
        ));

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
                context.emit(AppEvent::State(
                    StateEvent::TwoPhaseCommitPhaseOneCompleted {
                        state_id: transition.staging_id,
                        operation: transition.operation.clone(),
                    },
                ));
                journal
            }
            Err(e) => {
                context.emit(AppEvent::State(StateEvent::TwoPhaseCommitFailed {
                    state_id: transition.staging_id,
                    operation: transition.operation.clone(),
                    error: e.to_string(),
                    phase: "phase_one".to_string(),
                }));
                return Err(e);
            }
        };

        // Phase 2: Execute filesystem swap and finalize
        context.emit(AppEvent::State(
            StateEvent::TwoPhaseCommitPhaseTwoStarting {
                state_id: transition.staging_id,
                operation: transition.operation.clone(),
            },
        ));

        match self
            .state_manager
            .execute_filesystem_swap_and_finalize(journal)
            .await
        {
            Ok(()) => {
                context.emit(AppEvent::State(
                    StateEvent::TwoPhaseCommitPhaseTwoCompleted {
                        state_id: transition.staging_id,
                        operation: transition.operation.clone(),
                    },
                ));
            }
            Err(e) => {
                context.emit(AppEvent::State(StateEvent::TwoPhaseCommitFailed {
                    state_id: transition.staging_id,
                    operation: transition.operation.clone(),
                    error: e.to_string(),
                    phase: "phase_two".to_string(),
                }));
                return Err(e);
            }
        }

        // Emit 2PC completion event
        context.emit(AppEvent::State(StateEvent::TwoPhaseCommitCompleted {
            state_id: transition.staging_id,
            parent_state_id: parent_id,
            operation: transition.operation.clone(),
        }));

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

        context.emit(AppEvent::State(StateEvent::Initializing {
            state_id: transition.staging_id,
            operation: "Creating new state".to_string(),
            estimated_duration: None,
        }));

        // Clone current state to staging directory
        self.create_staging_directory(&self.live_path, &transition.staging_path)
            .await?;

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
            store,
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

        // Execute two-phase commit
        self.execute_two_phase_commit(&transition, context).await?;

        context.emit(AppEvent::State(StateEvent::TransitionCompleted {
            from: transition.parent_id.unwrap_or_default(),
            to: transition.staging_id,
            operation: "install".to_string(),
            duration: std::time::Duration::from_secs(0), // TODO: Track actual duration
        }));

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
            let _ = sender.send(AppEvent::General(GeneralEvent::DebugLog {
                message: format!("Linking package {} to staging", package_id.name),
                context: std::collections::HashMap::new(),
            }));
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
            let _ = sender.send(AppEvent::General(GeneralEvent::DebugLog {
                message: format!(
                    "Linked {} files/directories for package {}",
                    file_paths.len(),
                    package_id.name
                ),
                context: std::collections::HashMap::new(),
            }));
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

        // Execute two-phase commit
        self.execute_two_phase_commit(&transition, context).await?;

        context.emit(AppEvent::State(StateEvent::TransitionCompleted {
            from: transition.parent_id.unwrap_or_default(),
            to: transition.staging_id,
            operation: "uninstall".to_string(),
            duration: std::time::Duration::from_secs(0), // TODO: Track actual duration
        }));

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

        // Detect if this is a Python package for later cleanup
        let python_package_dir = self.detect_python_package_directory(&file_paths);

        // Always do file-by-file removal first to remove all tracked files (including wrapper scripts)
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

        // After removing all tracked files, clean up any remaining Python runtime artifacts
        if let Some(python_dir) = python_package_dir {
            let python_staging_dir = transition.staging_path.join(&python_dir);

            if python_staging_dir.exists() {
                // Check if directory still has content (runtime artifacts)
                if let Ok(mut entries) = tokio::fs::read_dir(&python_staging_dir).await {
                    if entries.next_entry().await?.is_some() {
                        // Directory is not empty, remove remaining runtime artifacts
                        tokio::fs::remove_dir_all(&python_staging_dir)
                            .await
                            .map_err(|e| InstallError::FilesystemError {
                                operation: "cleanup_python_runtime_artifacts".to_string(),
                                path: python_staging_dir.display().to_string(),
                                message: e.to_string(),
                            })?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Detect if this is a Python package and return the directory to remove
    ///
    /// Python packages are isolated in `/opt/pm/live/python/<package_name>/` directories.
    /// This method examines file paths to find the Python package directory.
    fn detect_python_package_directory(&self, file_paths: &[String]) -> Option<String> {
        for file_path in file_paths {
            // Look for files under python/ directory structure
            if let Some(stripped) = file_path.strip_prefix("python/") {
                // Extract the package directory (e.g., "ansible/" from "python/ansible/lib/...")
                if let Some(slash_pos) = stripped.find('/') {
                    let package_dir = format!("python/{}", &stripped[..slash_pos]);
                    return Some(package_dir);
                } else if !stripped.is_empty() {
                    // Handle case where the path is just "python/package_name"
                    let package_dir = format!("python/{}", stripped);
                    return Some(package_dir);
                }
            }
        }
        None
    }

    // Removed remove_package_venv - Python packages are now handled like regular packages

    /// Rollback to a previous state
    ///
    /// # Errors
    ///
    /// Returns an error if the target state doesn't exist, filesystem swap fails,
    /// or database update fails.
    pub async fn rollback(&mut self, target_state_id: Uuid) -> Result<(), Error> {
        // Reconstructive rollback: clone live -> staging, apply diffs current -> target, commit 2PC
        let current_state_id = self.state_manager.get_current_state_id().await?;

        // Setup state transition and staging directory
        let mut transition =
            StateTransition::new(&self.state_manager, "rollback".to_string()).await?;
        // Clone current live to staging dir
        self.create_staging_directory(&self.live_path, &transition.staging_path)
            .await?;

        // Load current and target package sets
        let current_packages = self
            .state_manager
            .get_installed_packages_in_state(&current_state_id)
            .await?;
        let target_packages = self
            .state_manager
            .get_installed_packages_in_state(&target_state_id)
            .await?;

        let current_map: HashMap<String, sps2_state::models::Package> = current_packages
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect();
        let target_map: HashMap<String, sps2_state::models::Package> = target_packages
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect();

        let current_names: HashSet<String> = current_map.keys().cloned().collect();
        let target_names: HashSet<String> = target_map.keys().cloned().collect();

        // Determine removals: present in current but not in target, or version differs
        let mut to_remove: Vec<sps2_state::models::Package> = Vec::new();
        for name in &current_names {
            if let Some(cur) = current_map.get(name) {
                match target_map.get(name) {
                    None => to_remove.push(cur.clone()),
                    Some(tgt) => {
                        if tgt.version != cur.version {
                            to_remove.push(cur.clone());
                        }
                    }
                }
            }
        }

        // Remove packages from staging
        for pkg in &to_remove {
            self.remove_package_from_staging(&mut transition, pkg)
                .await?;
        }

        // Determine additions/changes: present in target but not in current, or version differs
        let mut to_add: Vec<sps2_state::models::Package> = Vec::new();
        for name in &target_names {
            if let Some(tgt) = target_map.get(name) {
                match current_map.get(name) {
                    None => to_add.push(tgt.clone()),
                    Some(cur) => {
                        if tgt.version != cur.version {
                            to_add.push(tgt.clone());
                        }
                    }
                }
            }
        }

        // Link target packages into staging and stage DB refs
        for pkg in &to_add {
            // Resolve store path by hash
            let hash = sps2_hash::Hash::from_hex(&pkg.hash).map_err(|e| {
                sps2_errors::Error::internal(format!("invalid hash {}: {e}", pkg.hash))
            })?;
            let store_path = self.store.package_path(&hash);

            // Link files
            let pkg_id = sps2_resolver::PackageId::new(pkg.name.clone(), pkg.version());
            self.link_package_to_staging(&mut transition, &store_path, &pkg_id)
                .await?;

            // Stage package reference for DB
            let package_ref = PackageRef {
                state_id: transition.staging_id,
                package_id: pkg_id,
                hash: pkg.hash.clone(),
                size: pkg.size,
            };
            transition.package_refs.push(package_ref);

            // Ensure package_map is populated for this package (best-effort)
            let _ = self
                .state_manager
                .add_package_map(&pkg.name, &pkg.version, &pkg.hash)
                .await;
        }

        // Carry forward unchanged packages to keep DB references (no relinking needed)
        // Exclude packages whose names are in to_remove or to_add
        let mut exclude_ids: Vec<PackageId> = Vec::with_capacity(to_remove.len() + to_add.len());
        for pkg in &to_remove {
            exclude_ids.push(PackageId::new(pkg.name.clone(), pkg.version()));
        }
        for pkg in &to_add {
            exclude_ids.push(PackageId::new(pkg.name.clone(), pkg.version()));
        }

        self.carry_forward_packages(&mut transition, &exclude_ids)
            .await?;

        // Two-phase commit without event context
        let transition_data = sps2_state::TransactionData {
            package_refs: &transition.package_refs,
            package_files: &transition.package_files,
            file_references: &transition.file_references,
            pending_file_hashes: &transition.pending_file_hashes,
        };

        let parent_id = transition.parent_id.unwrap_or(current_state_id);
        let journal = self
            .state_manager
            .prepare_transaction(
                &transition.staging_id,
                &parent_id,
                &transition.staging_path,
                &transition.operation,
                &transition_data,
            )
            .await?;

        self.state_manager
            .execute_filesystem_swap_and_finalize(journal)
            .await?;

        Ok(())
    }

    /// Rollback by moving active to an existing target state without creating a new state row
    ///
    /// # Errors
    ///
    /// Returns an error if staging or filesystem operations fail.
    pub async fn rollback_move_to_state(&mut self, target_state_id: Uuid) -> Result<(), Error> {
        let current_state_id = self.state_manager.get_current_state_id().await?;

        // Setup staging and clone current live
        let mut transition =
            StateTransition::new(&self.state_manager, "rollback".to_string()).await?;
        self.create_staging_directory(&self.live_path, &transition.staging_path)
            .await?;

        // Compute diffs current -> target using DB
        let current_packages = self
            .state_manager
            .get_installed_packages_in_state(&current_state_id)
            .await?;
        let target_packages = self
            .state_manager
            .get_installed_packages_in_state(&target_state_id)
            .await?;

        let current_map: HashMap<String, sps2_state::models::Package> = current_packages
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect();
        let target_map: HashMap<String, sps2_state::models::Package> = target_packages
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect();

        // Remove anything not in target or with version change
        for (name, cur) in &current_map {
            match target_map.get(name) {
                None => {
                    self.remove_package_from_staging(&mut transition, cur)
                        .await?;
                }
                Some(tgt) if tgt.version != cur.version => {
                    self.remove_package_from_staging(&mut transition, cur)
                        .await?;
                }
                _ => {}
            }
        }

        // Add/link anything present in target and missing/different in current
        for (name, tgt) in &target_map {
            let needs_add = match current_map.get(name) {
                None => true,
                Some(cur) => cur.version != tgt.version,
            };
            if needs_add {
                let hash = sps2_hash::Hash::from_hex(&tgt.hash).map_err(|e| {
                    sps2_errors::Error::internal(format!("invalid hash {}: {e}", tgt.hash))
                })?;
                let store_path = self.store.package_path(&hash);
                let pkg_id = sps2_resolver::PackageId::new(tgt.name.clone(), tgt.version());
                self.link_package_to_staging(&mut transition, &store_path, &pkg_id)
                    .await?;
            }
        }

        // Journal and finalize: mark the target state as new active
        let journal = sps2_types::state::TransactionJournal {
            new_state_id: target_state_id,
            parent_state_id: current_state_id,
            staging_path: transition.staging_path.clone(),
            phase: sps2_types::state::TransactionPhase::Prepared,
            operation: "rollback".to_string(),
        };
        self.state_manager.write_journal(&journal).await?;
        self.state_manager
            .execute_filesystem_swap_and_finalize(journal)
            .await?;

        // Ensure the target state is visible in base history
        self.state_manager.unprune_state(&target_state_id).await?;

        Ok(())
    }
}
