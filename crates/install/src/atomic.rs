//! Atomic installation operations using APFS clonefile and state transitions

use crate::{InstallContext, InstallResult, StagingManager};
use sps2_errors::{Error, InstallError};
use sps2_events::Event;
use sps2_resolver::{PackageId, ResolvedNode};
use sps2_state::{PackageRef, StateManager};
use sps2_store::PackageStore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
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
        let staging_manager = StagingManager::new(store.clone()).await?;
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
    ) -> Result<InstallResult, Error> {
        // Create new state transition
        let mut transition = StateTransition::new(&self.state_manager).await?;

        if let Some(sender) = &context.event_sender {
            let _ = sender.send(Event::StateCreating {
                state_id: transition.staging_id,
            });
        }

        // Clone current state to staging directory
        transition.create_staging(&self.live_path)?;

        // Apply package changes to staging
        let mut result = InstallResult::new(transition.staging_id);

        for (package_id, node) in resolved_packages {
            self.install_package_to_staging(&mut transition, package_id, node, &mut result)
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
        result: &mut InstallResult,
    ) -> Result<(), Error> {
        match &node.action {
            sps2_resolver::NodeAction::Download => {
                // Package should already be in store from download phase
                let store_path = self
                    .store
                    .get_package_path(&package_id.name, &package_id.version)?;
                self.link_package_to_staging(transition, &store_path, package_id)
                    .await?;
            }
            sps2_resolver::NodeAction::Local => {
                if let Some(local_path) = &node.path {
                    // Use the new staging system for local packages
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
        let staging_dir =
            staging_guard
                .staging_dir()
                .ok_or_else(|| InstallError::AtomicOperationFailed {
                    message: "staging directory unavailable".to_string(),
                })?;

        // Add package to store from staging directory
        let _stored_package = self.store.add_package(local_path).await?;

        // Link package contents from staging to final location
        self.link_validated_staging_to_transition(transition, staging_dir, package_id)
            .await?;

        // Successfully processed - prevent cleanup
        let _staging_dir = staging_guard.take()?;

        Ok(())
    }

    /// Link validated staging directory contents to state transition
    async fn link_validated_staging_to_transition(
        &self,
        transition: &mut StateTransition,
        staging_dir: &crate::StagingDirectory,
        package_id: &PackageId,
    ) -> Result<(), Error> {
        let staging_files_path = staging_dir.files_path();
        let staging_prefix = &transition.staging_path;
        let mut file_paths = Vec::new();

        // Link files from validated staging to transition staging
        if staging_files_path.exists() {
            self.create_hardlinks_recursive_with_tracking(
                &staging_files_path,
                staging_prefix,
                &staging_files_path,
                &mut file_paths,
            )
            .await?;
        }

        // Store package reference to be added during commit
        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
            hash: "placeholder-hash".to_string(), // TODO: Get from staging validation
            size: 0,                              // TODO: Calculate from staging
        };
        transition.package_refs.push(package_ref);

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

    /// Link package from store to staging directory
    async fn link_package_to_staging(
        &self,
        transition: &mut StateTransition,
        store_path: &Path,
        package_id: &PackageId,
    ) -> Result<(), Error> {
        // Create hard links from store to staging directory and collect file paths
        let staging_prefix = &transition.staging_path;
        let mut file_paths = Vec::new();

        // Walk through store package contents and create hard links
        self.create_hardlinks_recursive_with_tracking(
            store_path,
            staging_prefix,
            store_path,
            &mut file_paths,
        )
        .await?;

        // Store package reference to be added during commit
        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
            hash: "placeholder-hash".to_string(), // TODO: Get from ResolvedNode
            size: 0,                              // TODO: Get from ResolvedNode
        };
        transition.package_refs.push(package_ref);

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

    /// Create hard links recursively without tracking (for legacy code)
    #[allow(dead_code)]
    async fn create_hardlinks_recursive(
        &self,
        source: &Path,
        dest_prefix: &Path,
    ) -> Result<(), Error> {
        let mut dummy_paths = Vec::new();
        self.create_hardlinks_recursive_with_tracking(source, dest_prefix, source, &mut dummy_paths)
            .await
    }

    /// Create hard links recursively and track file paths
    async fn create_hardlinks_recursive_with_tracking(
        &self,
        source: &Path,
        dest_prefix: &Path,
        root_source: &Path,
        file_paths: &mut Vec<(String, bool)>,
    ) -> Result<(), Error> {
        let mut entries = fs::read_dir(source).await?;

        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let file_name = entry.file_name();
            let dest_path = dest_prefix.join(&file_name);

            // Calculate relative path from store root
            let relative_path = entry_path.strip_prefix(root_source).map_err(|e| {
                InstallError::FilesystemError {
                    operation: "calculate_relative_path".to_string(),
                    path: entry_path.display().to_string(),
                    message: e.to_string(),
                }
            })?;

            if entry_path.is_dir() {
                // Create directory and recurse
                fs::create_dir_all(&dest_path).await?;

                // Record directory in file tracking
                file_paths.push((relative_path.display().to_string(), true));

                Box::pin(self.create_hardlinks_recursive_with_tracking(
                    &entry_path,
                    &dest_path,
                    root_source,
                    file_paths,
                ))
                .await?;
            } else {
                // Create hard link
                #[cfg(target_os = "macos")]
                {
                    // Use APFS hard link on macOS
                    Self::create_hard_link(&entry_path, &dest_path)?;
                }

                // Record file in file tracking
                file_paths.push((relative_path.display().to_string(), false));
            }
        }

        Ok(())
    }

    /// Create hard link (APFS-optimized on macOS)
    #[cfg(target_os = "macos")]
    fn create_hard_link(source: &Path, dest: &Path) -> Result<(), Error> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let source_c = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
            InstallError::FilesystemError {
                operation: "create_hard_link".to_string(),
                path: source.display().to_string(),
                message: "invalid path".to_string(),
            }
        })?;

        let dest_c = CString::new(dest.as_os_str().as_bytes()).map_err(|_| {
            InstallError::FilesystemError {
                operation: "create_hard_link".to_string(),
                path: dest.display().to_string(),
                message: "invalid path".to_string(),
            }
        })?;

        let result = unsafe { libc::link(source_c.as_ptr(), dest_c.as_ptr()) };

        if result != 0 {
            return Err(InstallError::FilesystemError {
                operation: "create_hard_link".to_string(),
                path: source.display().to_string(),
                message: format!("link failed with code {result}"),
            }
            .into());
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
        let target_path = self.state_manager.get_state_path(target_state_id)?;

        // Use true atomic swap to exchange target state with live directory
        sps2_root::atomic_swap(&target_path, &self.live_path)
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "rollback_atomic_swap".to_string(),
                path: target_path.display().to_string(),
                message: e.to_string(),
            })?;

        // Update active state in database
        self.state_manager.set_active_state(target_state_id).await?;

        Ok(())
    }
}

/// State transition for atomic operations
pub struct StateTransition {
    /// Staging state ID
    pub staging_id: Uuid,
    /// Parent state ID
    pub parent_id: Option<Uuid>,
    /// Staging directory path
    pub staging_path: PathBuf,
    /// State manager reference
    state_manager: StateManager,
    /// Package references to be added during commit
    package_refs: Vec<PackageRef>,
    /// Package files to be added during commit
    package_files: Vec<(String, String, String, bool)>, // (package_name, package_version, file_path, is_directory)
}

impl StateTransition {
    /// Create new state transition
    ///
    /// # Errors
    ///
    /// Returns an error if getting current state ID fails.
    pub async fn new(state_manager: &StateManager) -> Result<Self, Error> {
        let staging_id = Uuid::new_v4();
        let parent_id = state_manager.get_current_state_id().await?;
        let staging_path = state_manager
            .state_path()
            .join(format!("staging-{staging_id}"));

        Ok(Self {
            staging_id,
            parent_id: Some(parent_id),
            staging_path,
            state_manager: state_manager.clone(),
            package_refs: Vec::new(),
            package_files: Vec::new(),
        })
    }

    /// Create staging directory as APFS clone
    ///
    /// # Errors
    ///
    /// Returns an error if APFS clonefile system call fails.
    pub fn create_staging(&self, live_path: &Path) -> Result<(), Error> {
        #[cfg(target_os = "macos")]
        {
            if live_path.exists() {
                // Use APFS clonefile for instant, space-efficient copy
                Self::apfs_clonefile(live_path, &self.staging_path)?;
            } else {
                // Create empty staging directory for fresh installation
                std::fs::create_dir_all(&self.staging_path).map_err(|e| {
                    InstallError::FilesystemError {
                        operation: "create_staging_dir".to_string(),
                        path: self.staging_path.display().to_string(),
                        message: e.to_string(),
                    }
                })?;
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            if live_path.exists() {
                // Fallback recursive copy
                tokio::task::block_in_place(|| {
                    std::fs::create_dir_all(&self.staging_path).map_err(|e| {
                        InstallError::FilesystemError {
                            operation: "create_staging_dir".to_string(),
                            path: self.staging_path.display().to_string(),
                            message: e.to_string(),
                        }
                    })
                })?;
                // TODO: implement recursive copy
            } else {
                // Create empty staging directory for fresh installation
                std::fs::create_dir_all(&self.staging_path).map_err(|e| {
                    InstallError::FilesystemError {
                        operation: "create_staging_dir".to_string(),
                        path: self.staging_path.display().to_string(),
                        message: e.to_string(),
                    }
                })?;
            }
        }

        Ok(())
    }

    /// APFS clonefile implementation
    #[cfg(target_os = "macos")]
    fn apfs_clonefile(source: &Path, dest: &Path) -> Result<(), Error> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        // macOS clonefile syscall number
        const SYS_CLONEFILE: libc::c_int = 462;

        let source_c = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
            InstallError::FilesystemError {
                operation: "clonefile".to_string(),
                path: source.display().to_string(),
                message: "invalid source path".to_string(),
            }
        })?;

        let dest_c = CString::new(dest.as_os_str().as_bytes()).map_err(|_| {
            InstallError::FilesystemError {
                operation: "clonefile".to_string(),
                path: dest.display().to_string(),
                message: "invalid dest path".to_string(),
            }
        })?;

        // Call clonefile system call
        let result = unsafe {
            libc::syscall(
                SYS_CLONEFILE,
                source_c.as_ptr(),
                dest_c.as_ptr(),
                0i32, // flags
            )
        };

        if result != 0 {
            return Err(InstallError::FilesystemError {
                operation: "clonefile".to_string(),
                path: source.display().to_string(),
                message: format!("clonefile failed with code {result}"),
            }
            .into());
        }

        Ok(())
    }

    /// Fallback directory copy for non-APFS filesystems
    #[allow(dead_code)] // Will be used for non-APFS filesystem support
    async fn copy_directory_recursive(&self, source: &Path, dest: &Path) -> Result<(), Error> {
        fs::create_dir_all(dest).await?;

        let mut entries = fs::read_dir(source).await?;
        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let file_name = entry.file_name();
            let dest_path = dest.join(&file_name);

            if entry_path.is_dir() {
                Box::pin(self.copy_directory_recursive(&entry_path, &dest_path)).await?;
            } else {
                fs::copy(&entry_path, &dest_path).await?;
            }
        }

        Ok(())
    }

    /// Commit the state transition
    ///
    /// # Errors
    ///
    /// Returns an error if database transaction fails or filesystem operations fail.
    pub async fn commit(&self, live_path: &Path) -> Result<(), Error> {
        // Begin database transaction
        let mut tx = self.state_manager.begin_transaction().await?;

        // Record new state in database
        self.state_manager
            .create_state_with_tx(
                &mut tx,
                &self.staging_id,
                self.parent_id.as_ref(),
                "install",
            )
            .await?;

        // Add all stored package references to the database
        for package_ref in &self.package_refs {
            self.state_manager
                .add_package_ref_with_tx(&mut tx, package_ref)
                .await?;
        }

        // Add all stored package files to the database
        for (package_name, package_version, file_path, is_directory) in &self.package_files {
            sps2_state::queries::add_package_file(
                &mut tx,
                &self.staging_id,
                package_name,
                package_version,
                file_path,
                *is_directory,
            )
            .await?;
        }

        // Prepare archived path for current live directory
        let old_live_path = self
            .state_manager
            .state_path()
            .join(self.parent_id.unwrap_or_default().to_string());

        // True atomic swap using sps2_root::atomic_swap
        // This uses macOS renamex_np with RENAME_SWAP for single OS-level atomic operation
        if live_path.exists() {
            // Use atomic swap to exchange staging and live directories
            sps2_root::atomic_swap(&self.staging_path, live_path)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "atomic_swap".to_string(),
                    path: live_path.display().to_string(),
                    message: e.to_string(),
                })?;

            // Move the old live directory (now in staging_path) to archived location
            fs::rename(&self.staging_path, &old_live_path).await?;
        } else {
            // If live doesn't exist, just move staging to live (first install)
            fs::rename(&self.staging_path, live_path).await?;
        }

        // Update active state pointer
        self.state_manager
            .set_active_state_with_tx(&mut tx, self.staging_id)
            .await?;

        // Commit transaction
        tx.commit().await?;

        Ok(())
    }

    /// Clean up staging directory
    ///
    /// # Errors
    ///
    /// Returns an error if directory removal fails.
    pub async fn cleanup(&self) -> Result<(), Error> {
        if self.staging_path.exists() {
            fs::remove_dir_all(&self.staging_path).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs;

    #[tokio::test]
    async fn test_state_transition_id_generation() {
        // Test that transition generates unique IDs without database dependencies
        let temp = tempdir().unwrap();

        // Create a temporary staging transition manually to avoid database setup
        let staging_id = Uuid::new_v4();
        let staging_path = temp.path().join(format!("staging-{staging_id}"));

        assert!(!staging_id.is_nil());
        assert!(staging_path.display().to_string().contains("staging"));
    }

    #[tokio::test]
    async fn test_staging_directory_cleanup() {
        // Test staging directory cleanup without database dependencies
        let temp = tempdir().unwrap();
        let staging_path = temp.path().join("staging-test");

        // Create staging directory
        fs::create_dir_all(&staging_path).await.unwrap();
        assert!(staging_path.exists());

        // Clean up
        fs::remove_dir_all(&staging_path).await.unwrap();
        assert!(!staging_path.exists());
    }

    #[test]
    fn test_install_result() {
        let state_id = Uuid::new_v4();
        let mut result = InstallResult::new(state_id);

        assert_eq!(result.total_changes(), 0);

        let package_id = PackageId::new(
            "test-pkg".to_string(),
            sps2_types::Version::parse("1.0.0").unwrap(),
        );

        result.add_installed(package_id);
        assert_eq!(result.total_changes(), 1);
        assert_eq!(result.installed_packages.len(), 1);
    }

    #[tokio::test]
    async fn test_atomic_commit_swap() {
        // Test atomic swap behavior during commit
        let temp = tempdir().unwrap();
        let base_path = temp.path();

        // Create mock directories
        let live_path = base_path.join("live");
        let staging_path = base_path.join("staging");
        let archived_path = base_path.join("archived");

        // Set up live directory with content
        fs::create_dir_all(&live_path).await.unwrap();
        fs::create_dir_all(live_path.join("bin")).await.unwrap();
        fs::write(live_path.join("bin/old-app"), b"old version")
            .await
            .unwrap();

        // Set up staging directory with new content
        fs::create_dir_all(&staging_path).await.unwrap();
        fs::create_dir_all(staging_path.join("bin")).await.unwrap();
        fs::write(staging_path.join("bin/new-app"), b"new version")
            .await
            .unwrap();
        fs::write(staging_path.join("bin/old-app"), b"updated version")
            .await
            .unwrap();

        // Perform atomic swap
        sps2_root::atomic_swap(&staging_path, &live_path)
            .await
            .unwrap();

        // Verify swap occurred
        assert!(live_path.join("bin/new-app").exists());
        assert!(staging_path.join("bin/old-app").exists());

        // Verify content was swapped correctly
        let new_content = fs::read(live_path.join("bin/new-app")).await.unwrap();
        assert_eq!(new_content, b"new version");

        let old_content = fs::read(staging_path.join("bin/old-app")).await.unwrap();
        assert_eq!(old_content, b"old version");

        // Clean up staged directory (simulating archive step)
        fs::rename(&staging_path, &archived_path).await.unwrap();
        assert!(archived_path.exists());
        assert!(!staging_path.exists());
    }

    #[tokio::test]
    async fn test_atomic_rollback_swap() {
        // Test atomic swap behavior during rollback
        let temp = tempdir().unwrap();
        let base_path = temp.path();

        // Create mock directories
        let live_path = base_path.join("live");
        let backup_path = base_path.join("backup");

        // Set up live directory (current state)
        fs::create_dir_all(&live_path).await.unwrap();
        fs::create_dir_all(live_path.join("bin")).await.unwrap();
        fs::write(live_path.join("bin/app"), b"current version")
            .await
            .unwrap();

        // Set up backup directory (rollback target)
        fs::create_dir_all(&backup_path).await.unwrap();
        fs::create_dir_all(backup_path.join("bin")).await.unwrap();
        fs::write(backup_path.join("bin/app"), b"previous version")
            .await
            .unwrap();

        // Perform atomic swap for rollback
        sps2_root::atomic_swap(&backup_path, &live_path)
            .await
            .unwrap();

        // Verify rollback occurred
        let live_content = fs::read(live_path.join("bin/app")).await.unwrap();
        assert_eq!(live_content, b"previous version");

        let backup_content = fs::read(backup_path.join("bin/app")).await.unwrap();
        assert_eq!(backup_content, b"current version");
    }

    #[tokio::test]
    async fn test_rapid_operations() {
        // Test that rapid install/uninstall operations maintain consistency
        let temp = tempdir().unwrap();
        let base_path = temp.path();

        let live_path = base_path.join("live");
        let staging1_path = base_path.join("staging1");
        let staging2_path = base_path.join("staging2");

        // Initial state
        fs::create_dir_all(&live_path).await.unwrap();
        fs::write(live_path.join("state.txt"), b"initial")
            .await
            .unwrap();

        // First operation
        fs::create_dir_all(&staging1_path).await.unwrap();
        fs::write(staging1_path.join("state.txt"), b"first_update")
            .await
            .unwrap();

        sps2_root::atomic_swap(&staging1_path, &live_path)
            .await
            .unwrap();

        let content = fs::read(live_path.join("state.txt")).await.unwrap();
        assert_eq!(content, b"first_update");

        // Second operation (rapid)
        fs::create_dir_all(&staging2_path).await.unwrap();
        fs::write(staging2_path.join("state.txt"), b"second_update")
            .await
            .unwrap();

        sps2_root::atomic_swap(&staging2_path, &live_path)
            .await
            .unwrap();

        let content = fs::read(live_path.join("state.txt")).await.unwrap();
        assert_eq!(content, b"second_update");

        // Verify intermediate state is preserved
        let first_content = fs::read(staging2_path.join("state.txt")).await.unwrap();
        assert_eq!(first_content, b"first_update");
    }

    #[tokio::test]
    async fn test_atomic_swap_consistency() {
        // Test that atomic swap maintains directory consistency
        let temp = tempdir().unwrap();
        let base_path = temp.path();

        let dir1 = base_path.join("dir1");
        let dir2 = base_path.join("dir2");

        // Create complex directory structures
        fs::create_dir_all(dir1.join("subdir/nested"))
            .await
            .unwrap();
        fs::create_dir_all(dir2.join("other/deeply/nested"))
            .await
            .unwrap();

        fs::write(dir1.join("file1.txt"), b"content1")
            .await
            .unwrap();
        fs::write(dir1.join("subdir/file2.txt"), b"subcontent1")
            .await
            .unwrap();
        fs::write(dir2.join("file3.txt"), b"content2")
            .await
            .unwrap();
        fs::write(dir2.join("other/file4.txt"), b"othercontent2")
            .await
            .unwrap();

        // Perform swap
        sps2_root::atomic_swap(&dir1, &dir2).await.unwrap();

        // Verify complete structure was swapped
        assert!(dir1.join("file3.txt").exists());
        assert!(dir1.join("other/file4.txt").exists());
        assert!(dir2.join("file1.txt").exists());
        assert!(dir2.join("subdir/file2.txt").exists());

        // Verify content integrity
        let content1 = fs::read(dir2.join("file1.txt")).await.unwrap();
        assert_eq!(content1, b"content1");

        let content2 = fs::read(dir1.join("file3.txt")).await.unwrap();
        assert_eq!(content2, b"content2");
    }

    #[tokio::test]
    async fn test_first_install_without_existing_live() {
        // Test first install when live directory doesn't exist
        let temp = tempdir().unwrap();
        let base_path = temp.path();

        let live_path = base_path.join("live");
        let staging_path = base_path.join("staging");

        // Set up staging directory (no live directory exists yet)
        fs::create_dir_all(&staging_path).await.unwrap();
        fs::create_dir_all(staging_path.join("bin")).await.unwrap();
        fs::write(staging_path.join("bin/app"), b"first install")
            .await
            .unwrap();

        // For first install, just rename staging to live
        assert!(!live_path.exists());
        fs::rename(&staging_path, &live_path).await.unwrap();

        // Verify first install
        assert!(live_path.exists());
        assert!(!staging_path.exists());

        let content = fs::read(live_path.join("bin/app")).await.unwrap();
        assert_eq!(content, b"first install");
    }
}
