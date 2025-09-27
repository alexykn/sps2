//! Atomic installer implementation using APFS optimizations

use crate::atomic::transition::{StagingCreation, StagingMode, StateTransition};
use std::sync::Arc;
// Removed Python venv handling - Python packages are now handled like regular packages
use crate::{InstallContext, InstallResult, PreparedPackage, StagingManager};
use sps2_errors::{Error, InstallError};
use sps2_events::events::{StateTransitionContext, TransitionSummary};
use sps2_events::{
    AppEvent, EventEmitter, EventSender, FailureContext, GeneralEvent, StateEvent, UninstallEvent,
};
use sps2_hash::Hash;
use sps2_platform::{core::PlatformContext, PlatformManager};

use sps2_resolver::{PackageId, ResolvedNode};
use sps2_state::{file_queries_runtime, PackageRef, StateManager};
use sps2_store::{PackageStore, StoredPackage};
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::path::{Path, PathBuf};
use std::time::Instant;
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
        allow_clone: bool,
    ) -> Result<StagingCreation, Error> {
        let (platform, ctx) = self.create_platform_context();
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

        let live_exists = platform.filesystem().exists(&ctx, live_path).await;

        if allow_clone && live_exists {
            match platform
                .filesystem()
                .clone_directory(&ctx, live_path, staging_path)
                .await
            {
                Ok(()) => {
                    return Ok(StagingCreation {
                        mode: StagingMode::Cloned,
                        clone_attempted: true,
                        clone_error: None,
                    });
                }
                Err(e) => {
                    // Fallback to fresh staging if clone fails
                    let _ = platform
                        .filesystem()
                        .remove_dir_all(&ctx, staging_path)
                        .await;
                    platform
                        .filesystem()
                        .create_dir_all(&ctx, staging_path)
                        .await
                        .map_err(|err| InstallError::FilesystemError {
                            operation: "create_dir_all".to_string(),
                            path: staging_path.display().to_string(),
                            message: err.to_string(),
                        })?;

                    return Ok(StagingCreation {
                        mode: StagingMode::Fresh,
                        clone_attempted: true,
                        clone_error: Some(e.to_string()),
                    });
                }
            }
        } else {
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

        Ok(StagingCreation {
            mode: StagingMode::Fresh,
            clone_attempted: allow_clone && live_exists,
            clone_error: None,
        })
    }
    /// Execute two-phase commit flow for a transition
    async fn execute_two_phase_commit<T: EventEmitter>(
        &self,
        transition: &StateTransition,
        context: &T,
    ) -> Result<(), Error> {
        let source = transition.parent_id;
        let target = transition.staging_id;

        let transition_context = StateTransitionContext {
            operation: transition.operation.clone(),
            source,
            target,
        };
        let transition_start = Instant::now();

        context.emit(AppEvent::State(StateEvent::TransitionStarted {
            context: transition_context.clone(),
        }));

        let transition_data = sps2_state::TransactionData {
            package_refs: &transition.package_refs,
            file_references: &transition.file_references,
            pending_file_hashes: &transition.pending_file_hashes,
        };

        let journal = match self
            .state_manager
            .prepare_transaction(
                &transition.staging_id,
                &transition.parent_id.unwrap_or_default(),
                &transition.staging_path,
                &transition.operation,
                &transition_data,
            )
            .await
        {
            Ok(journal) => journal,
            Err(e) => {
                let failure = FailureContext::from_error(&e);
                context.emit(AppEvent::State(StateEvent::TransitionFailed {
                    context: transition_context.clone(),
                    failure,
                }));
                return Err(e);
            }
        };

        if let Err(e) = self
            .state_manager
            .execute_filesystem_swap_and_finalize(journal)
            .await
        {
            let failure = FailureContext::from_error(&e);
            context.emit(AppEvent::State(StateEvent::TransitionFailed {
                context: transition_context.clone(),
                failure,
            }));
            return Err(e);
        }

        let summary = TransitionSummary {
            duration_ms: Some(
                u64::try_from(transition_start.elapsed().as_millis()).unwrap_or(u64::MAX),
            ),
        };

        context.emit(AppEvent::State(StateEvent::TransitionCompleted {
            context: transition_context,
            summary: Some(summary),
        }));

        Ok(())
    }

    /// Carry forward packages from parent state, excluding specified packages
    async fn carry_forward_packages(
        &self,
        transition: &mut StateTransition,
        parent_packages: &[sps2_state::models::Package],
        exclude_names: &HashSet<String>,
    ) -> Result<(), Error> {
        if transition.parent_id.is_some() {
            for pkg in parent_packages {
                if exclude_names.contains(&pkg.name) {
                    continue;
                }

                let hash = Hash::from_hex(&pkg.hash).map_err(|e| {
                    Error::from(InstallError::AtomicOperationFailed {
                        message: format!(
                            "invalid package hash for {}-{}: {e}",
                            pkg.name, pkg.version
                        ),
                    })
                })?;

                let store_path = self.store.package_path(&hash);
                let package_id = PackageId::new(pkg.name.clone(), pkg.version());

                let package_ref = PackageRef {
                    state_id: transition.staging_id,
                    package_id: package_id.clone(),
                    hash: pkg.hash.clone(),
                    size: pkg.size,
                };
                transition.package_refs.push(package_ref);

                if matches!(transition.staging_mode, StagingMode::Fresh) {
                    // Fresh staging needs actual files linked, but we skip recording hashes
                    // because the package version already has file metadata.
                    self.link_package_to_staging(transition, &store_path, &package_id, false)
                        .await?;
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

        // Initialize staging directory
        let creation = self
            .create_staging_directory(&self.live_path, &transition.staging_path, true)
            .await?;
        transition.staging_mode = creation.mode;

        let mut message = format!(
            "Prepared staging directory at: {} (mode: {:?})",
            transition.staging_path.display(),
            creation.mode
        );
        if creation.clone_attempted && creation.mode == StagingMode::Fresh {
            if let Some(err) = creation.clone_error {
                message.push_str("; clone fallback due to: ");
                message.push_str(&err);
            } else {
                message.push_str("; clone not available");
            }
        }
        context.emit_debug(message);

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

        // Collect the current state's packages so we can carry forward untouched entries and
        // detect in-place upgrades cleanly.
        let parent_packages = if let Some(parent_id) = transition.parent_id {
            self.state_manager
                .get_installed_packages_in_state(&parent_id)
                .await?
        } else {
            Vec::new()
        };
        let parent_lookup: HashMap<String, sps2_state::models::Package> = parent_packages
            .iter()
            .cloned()
            .map(|pkg| (pkg.name.clone(), pkg))
            .collect();

        // APFS clonefile already copies all existing packages, so we don't need to re-link them.
        // We only need to carry forward the package references and file information in the database.
        let exclude_names: HashSet<String> = resolved_packages
            .keys()
            .map(|pkg| pkg.name.clone())
            .collect();
        self.carry_forward_packages(&mut transition, &parent_packages, &exclude_names)
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
                parent_lookup.get(&package_id.name),
                &mut result,
            )
            .await?;
        }

        // Execute two-phase commit
        self.execute_two_phase_commit(&transition, context).await?;

        Ok(result)
    }

    /// Install a single package to staging directory
    async fn install_package_to_staging(
        &self,
        transition: &mut StateTransition,
        package_id: &PackageId,
        node: &ResolvedNode,
        prepared_package: Option<&PreparedPackage>,
        prior_package: Option<&sps2_state::models::Package>,
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
        let store_hash_hex = hash.to_hex();
        let package_hash_hex = prepared.package_hash.as_ref().map(sps2_hash::Hash::to_hex);

        let mut was_present = false;
        let mut version_changed = false;
        if let Some(existing) = prior_package {
            was_present = true;
            let existing_version = existing.version();
            if existing_version != package_id.version {
                version_changed = true;
                self.remove_package_from_staging(transition, existing)
                    .await?;
            }
        }

        // Load package from the prepared store path
        let _stored_package = StoredPackage::load(store_path).await?;

        // Ensure store_refs entry exists before adding to package_map
        self.state_manager
            .ensure_store_ref(&store_hash_hex, size as i64)
            .await?;

        // Ensure package is in package_map for future lookups
        self.state_manager
            .add_package_map(
                &package_id.name,
                &package_id.version.to_string(),
                &store_hash_hex,
                package_hash_hex.as_deref(),
            )
            .await?;

        // Link package files to staging
        let _ = self
            .link_package_to_staging(transition, store_path, package_id, true)
            .await?;

        // Add the package reference
        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
            hash: store_hash_hex.clone(),
            size: size as i64,
        };
        transition.package_refs.push(package_ref);

        if was_present && version_changed {
            result.add_updated(package_id.clone());
        } else {
            result.add_installed(package_id.clone());
        }
        Ok(())
    }

    /// Link package from store to staging directory
    async fn link_package_to_staging(
        &self,
        transition: &mut StateTransition,
        store_path: &Path,
        package_id: &PackageId,
        record_hashes: bool,
    ) -> Result<bool, Error> {
        let staging_prefix = &transition.staging_path;

        // Load the stored package
        let stored_package = StoredPackage::load(store_path).await?;

        // Link files from store to staging
        if let Some(sender) = &transition.event_sender {
            sender.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: format!("Linking package {} to staging", package_id.name),
                context: std::collections::HashMap::new(),
            }));
        }

        stored_package.link_to(staging_prefix).await?;

        let mut had_file_hashes = false;
        let mut linked_entry_count = 0usize;
        // Collect file paths for database tracking AND store file hash info
        if let Some(file_hashes) = stored_package.file_hashes() {
            had_file_hashes = true;
            linked_entry_count = file_hashes.len();
            // Store the file hash information for later use when we have package IDs
            if record_hashes {
                transition
                    .pending_file_hashes
                    .push((package_id.clone(), file_hashes.to_vec()));
            }
        }

        // Debug what was linked
        if let Some(sender) = &transition.event_sender {
            sender.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: format!(
                    "Linked {} files/directories for package {}",
                    linked_entry_count, package_id.name
                ),
                context: std::collections::HashMap::new(),
            }));
        }

        Ok(had_file_hashes)
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

        for pkg in packages_to_remove {
            context.emit(AppEvent::Uninstall(UninstallEvent::Started {
                package: pkg.name.clone(),
                version: pkg.version.clone(),
            }));
        }

        // Remove packages from staging and track them in result
        let parent_packages = if let Some(parent_id) = transition.parent_id {
            self.state_manager
                .get_installed_packages_in_state(&parent_id)
                .await?
        } else {
            Vec::new()
        };

        for pkg in &parent_packages {
            // Check if this package should be removed
            let should_remove = packages_to_remove
                .iter()
                .any(|remove_pkg| remove_pkg.name == pkg.name);

            if should_remove {
                result.add_removed(PackageId::new(pkg.name.clone(), pkg.version()));
                if transition.staging_mode == StagingMode::Cloned {
                    self.remove_package_from_staging(&mut transition, pkg)
                        .await?;
                }
                context.emit_debug(format!("Removed package {} from staging", pkg.name));
            }
        }

        // Carry forward packages that are not being removed
        let exclude_names: HashSet<String> = packages_to_remove
            .iter()
            .map(|pkg| pkg.name.clone())
            .collect();
        self.carry_forward_packages(&mut transition, &parent_packages, &exclude_names)
            .await?;

        // Execute two-phase commit
        self.execute_two_phase_commit(&transition, context).await?;

        for pkg in &result.removed_packages {
            context.emit(AppEvent::Uninstall(UninstallEvent::Completed {
                package: pkg.name.clone(),
                version: pkg.version.clone(),
                files_removed: 0,
            }));
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
        let state_id = Uuid::parse_str(&package.state_id).map_err(|e| {
            InstallError::AtomicOperationFailed {
                message: format!(
                    "failed to parse associated state ID for package {}: {e}",
                    package.name
                ),
            }
        })?;

        let mut tx = self.state_manager.begin_transaction().await?;
        let entries = file_queries_runtime::get_package_file_entries_by_name(
            &mut tx,
            &state_id,
            &package.name,
            &package.version,
        )
        .await?;
        tx.commit().await?;

        let file_paths: Vec<String> = entries
            .into_iter()
            .map(|entry| entry.relative_path)
            .collect();

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
        let creation = self
            .create_staging_directory(&self.live_path, &transition.staging_path, true)
            .await?;
        let target_packages = self
            .state_manager
            .get_installed_packages_in_state(&target_state_id)
            .await?;
        transition.staging_mode = creation.mode;

        if let Some(sender) = &transition.event_sender {
            let mut msg = format!(
                "Rollback staging prepared at {} (mode {:?})",
                transition.staging_path.display(),
                creation.mode
            );
            if creation.clone_attempted && creation.mode == StagingMode::Fresh {
                if let Some(err) = creation.clone_error {
                    msg.push_str("; clone fallback due to: ");
                    msg.push_str(&err);
                } else {
                    msg.push_str("; clone not available");
                }
            }
            sender.emit(AppEvent::General(GeneralEvent::DebugLog {
                message: msg,
                context: std::collections::HashMap::new(),
            }));
        }

        if transition.staging_mode == StagingMode::Cloned {
            let current_packages = self
                .state_manager
                .get_installed_packages_in_state(&current_state_id)
                .await?;

            let current_map: HashMap<String, sps2_state::models::Package> = current_packages
                .into_iter()
                .map(|pkg| (pkg.name.clone(), pkg))
                .collect();
            let target_map: HashMap<String, &sps2_state::models::Package> = target_packages
                .iter()
                .map(|pkg| (pkg.name.clone(), pkg))
                .collect();

            for (name, cur_pkg) in &current_map {
                match target_map.get(name) {
                    Some(tgt_pkg) if tgt_pkg.version == cur_pkg.version => {}
                    _ => {
                        self.remove_package_from_staging(&mut transition, cur_pkg)
                            .await?;
                        if let Some(sender) = &transition.event_sender {
                            sender.emit(AppEvent::General(GeneralEvent::DebugLog {
                                message: format!(
                                    "Rollback removed package {}@{} from staging",
                                    cur_pkg.name, cur_pkg.version
                                ),
                                context: std::collections::HashMap::new(),
                            }));
                        }
                    }
                }
            }
        }
        // Compute diffs current -> target using DB
        let target_packages = self
            .state_manager
            .get_installed_packages_in_state(&target_state_id)
            .await?;
        for tgt in &target_packages {
            let hash = Hash::from_hex(&tgt.hash).map_err(|e| {
                sps2_errors::Error::internal(format!("invalid hash {}: {e}", tgt.hash))
            })?;
            let store_path = self.store.package_path(&hash);
            let pkg_id = sps2_resolver::PackageId::new(tgt.name.clone(), tgt.version());
            let _ = self
                .link_package_to_staging(&mut transition, &store_path, &pkg_id, false)
                .await?;
        }

        // Journal and finalize: mark the target state as new active
        // Note: Refcount semantics are handled centrally by StateManager::prepare_transaction.
        // The installer only performs the filesystem swap and DB finalization steps.

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

        // After switching active state, synchronize DB refcounts to match the target state exactly
        let _ = self
            .state_manager
            .sync_refcounts_to_state(&target_state_id)
            .await?;

        // Ensure the target state is visible in base history
        self.state_manager.unprune_state(&target_state_id).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_store::create_package;
    use sps2_types::{Arch, Manifest, Version};
    use std::collections::HashMap;
    use tempfile::TempDir;
    use tokio::fs as afs;

    async fn mk_env() -> (TempDir, StateManager, sps2_store::PackageStore) {
        let td = TempDir::new().expect("td");
        let state = StateManager::new(td.path()).await.expect("state");
        let store_base = td.path().join("store");
        afs::create_dir_all(&store_base).await.unwrap();
        let store = sps2_store::PackageStore::new(store_base);
        (td, state, store)
    }

    async fn make_sp_and_add_to_store(
        store: &sps2_store::PackageStore,
        name: &str,
        version: &str,
        files: &[(&str, &str)],
    ) -> (
        sps2_hash::Hash,
        std::path::PathBuf,
        u64,
        Vec<sps2_hash::Hash>,
    ) {
        let td = TempDir::new().unwrap();
        let src = td.path().join("src");
        afs::create_dir_all(&src).await.unwrap();
        // manifest
        let v = Version::parse(version).unwrap();
        let m = Manifest::new(name.to_string(), &v, 1, &Arch::Arm64);
        let manifest_path = src.join("manifest.toml");
        sps2_store::manifest_io::write_manifest(&manifest_path, &m)
            .await
            .unwrap();
        // files under opt/pm/live
        for (rel, content) in files {
            let p = src.join("opt/pm/live").join(rel);
            if let Some(parent) = p.parent() {
                afs::create_dir_all(parent).await.unwrap();
            }
            afs::write(&p, content.as_bytes()).await.unwrap();
        }
        // create .sp
        let sp = td.path().join("pkg.sp");
        create_package(&src, &sp).await.unwrap();
        // add to store
        let stored = store.add_package(&sp).await.unwrap();
        let hash = stored.hash().unwrap();
        let path = store.package_path(&hash);
        let file_hashes: Vec<sps2_hash::Hash> = stored
            .file_hashes()
            .unwrap_or(&[])
            .iter()
            .map(|r| r.hash.clone())
            .collect();
        let size = afs::metadata(&sp).await.unwrap().len();
        (hash, path, size, file_hashes)
    }

    fn collect_relative_files(base: &std::path::Path) -> Vec<std::path::PathBuf> {
        fn walk(
            base: &std::path::Path,
            current: &std::path::Path,
            acc: &mut Vec<std::path::PathBuf>,
        ) -> std::io::Result<()> {
            for entry in std::fs::read_dir(current)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    walk(base, &path, acc)?;
                } else {
                    acc.push(path.strip_prefix(base).unwrap().to_path_buf());
                }
            }
            Ok(())
        }

        let mut acc = Vec::new();
        walk(base, base, &mut acc).unwrap();
        acc
    }

    async fn refcount_store(state: &StateManager, hex: &str) -> i64 {
        let mut tx = state.begin_transaction().await.unwrap();
        let all = sps2_state::queries::get_all_store_refs(&mut tx)
            .await
            .unwrap();
        all.into_iter()
            .find(|r| r.hash == hex)
            .map_or(0, |r| r.ref_count)
    }
    async fn refcount_file(state: &StateManager, hex: &str) -> i64 {
        let mut tx = state.begin_transaction().await.unwrap();
        let h = sps2_hash::Hash::from_hex(hex).unwrap();
        let row = sps2_state::file_queries_runtime::get_file_object(&mut tx, &h)
            .await
            .unwrap();
        row.map_or(0, |o| o.ref_count)
    }

    #[tokio::test]
    async fn cloned_staging_carries_forward_package_files() {
        let (_td, state, store) = mk_env().await;
        let (hash_a, path_a, size_a, _file_hashes_a) = make_sp_and_add_to_store(
            &store,
            "A",
            "1.0.0",
            &[("bin/a", "alpha"), ("share/doc.txt", "alpha docs")],
        )
        .await;
        let (hash_b, path_b, size_b, _file_hashes_b) = make_sp_and_add_to_store(
            &store,
            "B",
            "1.0.0",
            &[("bin/b", "beta"), ("share/readme.txt", "beta docs")],
        )
        .await;

        let mut installer = AtomicInstaller::new(state.clone(), store.clone())
            .await
            .unwrap();

        // Install package A
        let mut resolved_a: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_a = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved_a.insert(
            pid_a.clone(),
            ResolvedNode::local(
                pid_a.name.clone(),
                pid_a.version.clone(),
                path_a.clone(),
                vec![],
            ),
        );
        let mut prepared_a = HashMap::new();
        prepared_a.insert(
            pid_a.clone(),
            crate::PreparedPackage {
                hash: hash_a.clone(),
                size: size_a,
                store_path: path_a.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _ = installer
            .install(&ctx, &resolved_a, Some(&prepared_a))
            .await
            .unwrap();

        // Install package B (forces cloned staging when clone succeeds)
        let mut resolved_b: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_b = PackageId::new(
            "B".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved_b.insert(
            pid_b.clone(),
            ResolvedNode::local(
                pid_b.name.clone(),
                pid_b.version.clone(),
                path_b.clone(),
                vec![],
            ),
        );
        let mut prepared_b = HashMap::new();
        prepared_b.insert(
            pid_b.clone(),
            crate::PreparedPackage {
                hash: hash_b.clone(),
                size: size_b,
                store_path: path_b.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx_b = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _ = installer
            .install(&ctx_b, &resolved_b, Some(&prepared_b))
            .await
            .unwrap();

        let active_state = state.get_current_state_id().await.unwrap();
        let mut tx = state.begin_transaction().await.unwrap();
        let pkg_a_files = sps2_state::file_queries_runtime::get_package_file_entries_by_name(
            &mut tx,
            &active_state,
            "A",
            "1.0.0",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert!(
            !pkg_a_files.is_empty(),
            "package_files entries for package A should be preserved after cloned staging"
        );
    }

    #[tokio::test]
    async fn install_then_update_replaces_old_version() {
        let (_td, state, store) = mk_env().await;
        let (hash_v1, path_v1, size_v1, _file_hashes_v1) = make_sp_and_add_to_store(
            &store,
            "A",
            "1.0.0",
            &[("share/v1.txt", "v1"), ("bin/a", "binary")],
        )
        .await;
        let (hash_v2, path_v2, size_v2, _file_hashes_v2) = make_sp_and_add_to_store(
            &store,
            "A",
            "1.1.0",
            &[("share/v2.txt", "v2"), ("bin/a", "binary2")],
        )
        .await;

        let mut ai = AtomicInstaller::new(state.clone(), store.clone())
            .await
            .unwrap();

        // Initial install of v1
        let mut resolved: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_v1 = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved.insert(
            pid_v1.clone(),
            ResolvedNode::local(
                "A".to_string(),
                pid_v1.version.clone(),
                path_v1.clone(),
                vec![],
            ),
        );
        let mut prepared = HashMap::new();
        prepared.insert(
            pid_v1.clone(),
            crate::PreparedPackage {
                hash: hash_v1.clone(),
                size: size_v1,
                store_path: path_v1.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _ = ai.install(&ctx, &resolved, Some(&prepared)).await.unwrap();

        let live_path = state.live_path().to_path_buf();
        let files_after_install = collect_relative_files(&live_path);
        assert!(files_after_install
            .iter()
            .any(|p| p.ends_with("share/v1.txt")));
        let binary_rel_initial = files_after_install
            .iter()
            .find(|p| p.ends_with("bin/a"))
            .expect("binary present after initial install");
        assert_eq!(
            std::fs::read_to_string(live_path.join(binary_rel_initial)).unwrap(),
            "binary"
        );

        // Update to v2
        let mut resolved_update: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_v2 = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.1.0").unwrap(),
        );
        resolved_update.insert(
            pid_v2.clone(),
            ResolvedNode::local(
                "A".to_string(),
                pid_v2.version.clone(),
                path_v2.clone(),
                vec![],
            ),
        );
        let mut prepared_update = HashMap::new();
        prepared_update.insert(
            pid_v2.clone(),
            crate::PreparedPackage {
                hash: hash_v2.clone(),
                size: size_v2,
                store_path: path_v2.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let update_ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: true,
            force_download: false,
            event_sender: None,
        };
        let update_result = ai
            .install(&update_ctx, &resolved_update, Some(&prepared_update))
            .await
            .unwrap();

        assert!(update_result.installed_packages.is_empty());
        assert_eq!(update_result.updated_packages, vec![pid_v2.clone()]);
        assert!(update_result.removed_packages.is_empty());

        let installed = state.get_installed_packages().await.unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].version, "1.1.0");

        // Live directory should reflect the new version
        let files_after_update = collect_relative_files(&live_path);
        assert!(!files_after_update
            .iter()
            .any(|p| p.ends_with("share/v1.txt")));
        assert!(files_after_update
            .iter()
            .any(|p| p.ends_with("share/v2.txt")));
        let binary_rel_updated = files_after_update
            .iter()
            .find(|p| p.ends_with("bin/a"))
            .expect("binary present after update");
        assert_eq!(
            std::fs::read_to_string(live_path.join(binary_rel_updated)).unwrap(),
            "binary2"
        );

        assert_eq!(refcount_store(&state, &hash_v1.to_hex()).await, 0);
        assert!(refcount_store(&state, &hash_v2.to_hex()).await > 0);
    }

    #[tokio::test]
    async fn install_then_uninstall_updates_refcounts() {
        let (_td, state, store) = mk_env().await;
        let (hash, store_path, size, file_hashes) =
            make_sp_and_add_to_store(&store, "A", "1.0.0", &[("bin/x", "same"), ("share/a", "A")])
                .await;

        let mut ai = AtomicInstaller::new(state.clone(), store.clone())
            .await
            .unwrap();
        let mut resolved: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved.insert(
            pid.clone(),
            ResolvedNode::local(
                "A".to_string(),
                pid.version.clone(),
                store_path.clone(),
                vec![],
            ),
        );
        let mut prepared = HashMap::new();
        prepared.insert(
            pid.clone(),
            crate::PreparedPackage {
                hash: hash.clone(),
                size,
                store_path: store_path.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _res = ai.install(&ctx, &resolved, Some(&prepared)).await.unwrap();

        // After install
        assert!(refcount_store(&state, &hash.to_hex()).await > 0);
        for fh in &file_hashes {
            assert!(refcount_file(&state, &fh.to_hex()).await > 0);
        }

        // Uninstall package A
        let uctx = crate::UninstallContext {
            packages: vec!["A".to_string()],
            autoremove: false,
            force: true,
            event_sender: None,
        };
        let _u = ai
            .uninstall(
                &[PackageId::new(
                    "A".to_string(),
                    sps2_types::Version::parse("1.0.0").unwrap(),
                )],
                &uctx,
            )
            .await
            .unwrap();

        assert_eq!(refcount_store(&state, &hash.to_hex()).await, 0);
        for fh in &file_hashes {
            assert_eq!(refcount_file(&state, &fh.to_hex()).await, 0);
        }
    }

    #[tokio::test]
    async fn shared_file_uninstall_decrements_but_not_zero() {
        let (_td, state, store) = mk_env().await;
        // A and B share bin/x
        let (hash_a, path_a, size_a, file_hashes_a) = make_sp_and_add_to_store(
            &store,
            "A",
            "1.0.0",
            &[("bin/x", "same"), ("share/a", "AA")],
        )
        .await;
        let (hash_b, path_b, size_b, file_hashes_b) = make_sp_and_add_to_store(
            &store,
            "B",
            "1.0.0",
            &[("bin/x", "same"), ("share/b", "BB")],
        )
        .await;
        let h_same = file_hashes_a
            .iter()
            .find(|h| file_hashes_b.iter().any(|hb| hb == *h))
            .unwrap()
            .clone();

        let mut ai = AtomicInstaller::new(state.clone(), store.clone())
            .await
            .unwrap();

        // Install A then B
        let mut resolved: HashMap<PackageId, ResolvedNode> = HashMap::new();
        let pid_a = PackageId::new(
            "A".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        let pid_b = PackageId::new(
            "B".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );
        resolved.insert(
            pid_a.clone(),
            ResolvedNode::local(
                "A".to_string(),
                pid_a.version.clone(),
                path_a.clone(),
                vec![],
            ),
        );
        resolved.insert(
            pid_b.clone(),
            ResolvedNode::local(
                "B".to_string(),
                pid_b.version.clone(),
                path_b.clone(),
                vec![],
            ),
        );

        let mut prepared = HashMap::new();
        prepared.insert(
            pid_a.clone(),
            crate::PreparedPackage {
                hash: hash_a.clone(),
                size: size_a,
                store_path: path_a.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        prepared.insert(
            pid_b.clone(),
            crate::PreparedPackage {
                hash: hash_b.clone(),
                size: size_b,
                store_path: path_b.clone(),
                is_local: true,
                package_hash: None,
            },
        );
        let ctx = crate::InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        let _res = ai.install(&ctx, &resolved, Some(&prepared)).await.unwrap();

        // Uninstall A
        let uctx = crate::UninstallContext {
            packages: vec!["A".to_string()],
            autoremove: false,
            force: true,
            event_sender: None,
        };
        let _u = ai
            .uninstall(std::slice::from_ref(&pid_a), &uctx)
            .await
            .unwrap();

        // Shared file remains referenced by B
        assert!(refcount_file(&state, &h_same.to_hex()).await > 0);
    }
}
