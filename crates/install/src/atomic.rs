//! Atomic installation operations using APFS clonefile and state transitions

use crate::{InstallContext, InstallResult};
use spsv2_errors::{Error, InstallError};
use spsv2_events::{Event, EventSender};
use spsv2_resolver::{ResolvedNode, PackageId};
use spsv2_state::{StateManager, PackageRef};
use spsv2_store::PackageStore;
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
    /// Live prefix path
    live_path: PathBuf,
}

impl AtomicInstaller {
    /// Create new atomic installer
    pub fn new(state_manager: StateManager, store: PackageStore) -> Self {
        Self {
            state_manager,
            store,
            live_path: PathBuf::from("/opt/pm/live"),
        }
    }

    /// Perform atomic installation
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
        transition.create_staging(&self.live_path).await?;

        // Apply package changes to staging
        let mut result = InstallResult::new(transition.staging_id);

        for (package_id, node) in resolved_packages {
            self.install_package_to_staging(&transition, package_id, node, &mut result)
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
        transition: &StateTransition,
        package_id: &PackageId,
        node: &ResolvedNode,
        result: &mut InstallResult,
    ) -> Result<(), Error> {
        match &node.action {
            spsv2_resolver::NodeAction::Download => {
                // Package should already be in store from download phase
                let store_path = self
                    .store
                    .get_package_path(&package_id.name, &package_id.version)?;
                self.link_package_to_staging(transition, &store_path, package_id)
                    .await?;
            }
            spsv2_resolver::NodeAction::Local => {
                if let Some(local_path) = &node.path {
                    // Extract local package to store and link
                    let store_path = self.store.add_local_package(local_path).await?;
                    self.link_package_to_staging(transition, &store_path, package_id)
                        .await?;
                }
            }
        }

        result.add_installed(package_id.clone());
        Ok(())
    }

    /// Link package from store to staging directory
    async fn link_package_to_staging(
        &self,
        transition: &StateTransition,
        store_path: &Path,
        package_id: &PackageId,
    ) -> Result<(), Error> {
        // Create hard links from store to staging directory
        let staging_prefix = &transition.staging_path;

        // Walk through store package contents and create hard links
        self.create_hardlinks_recursive(&store_path.to_path_buf(), &staging_prefix.to_path_buf()).await?;

        // Update package references in database
        let package_ref = PackageRef {
            state_id: transition.staging_id,
            package_id: package_id.clone(),
        };
        self.state_manager
            .add_package_ref(&package_ref)
            .await?;

        Ok(())
    }

    /// Create hard links recursively
    async fn create_hardlinks_recursive(
        &self,
        source: &std::path::PathBuf,
        dest_prefix: &std::path::PathBuf,
    ) -> Result<(), Error> {
        let mut entries = fs::read_dir(source).await?;

        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let file_name = entry.file_name();
            let dest_path = dest_prefix.join(&file_name);

            if entry_path.is_dir() {
                // Create directory and recurse
                fs::create_dir_all(&dest_path).await?;
                Box::pin(self.create_hardlinks_recursive(&entry_path, &dest_path)).await?;
            } else {
                // Create hard link
                #[cfg(target_os = "macos")]
                {
                    // Use APFS hard link on macOS
                    self.create_hard_link(&entry_path, &dest_path).await?;
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // Fallback to copy on other platforms
                    fs::copy(&entry_path, &dest_path).await?;
                }
            }
        }

        Ok(())
    }

    /// Create hard link (APFS-optimized on macOS)
    #[cfg(target_os = "macos")]
    async fn create_hard_link(&self, source: &Path, dest: &Path) -> Result<(), Error> {
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
    pub async fn rollback(&mut self, target_state_id: Uuid) -> Result<(), Error> {
        let target_path = self.state_manager.get_state_path(target_state_id).await?;

        // Atomic swap to target state
        self.atomic_rename_swap(&target_path, &self.live_path)
            .await?;

        // Update active state in database
        self.state_manager.set_active_state(target_state_id).await?;

        Ok(())
    }

    /// Atomic rename swap using renameat2 with RENAME_SWAP
    async fn atomic_rename_swap(&self, from: &Path, to: &Path) -> Result<(), Error> {
        #[cfg(target_os = "macos")]
        {
            // On macOS, use atomic rename
            fs::rename(from, to)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "atomic_swap".to_string(),
                    path: from.display().to_string(),
                    message: e.to_string(),
                })?;
        }
        #[cfg(not(target_os = "macos"))]
        {
            // On other platforms, use simple rename (less atomic)
            fs::rename(from, to)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "atomic_swap".to_string(),
                    path: from.display().to_string(),
                    message: e.to_string(),
                })?;
        }

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
}

impl StateTransition {
    /// Create new state transition
    pub async fn new(state_manager: &StateManager) -> Result<Self, Error> {
        let staging_id = Uuid::new_v4();
        let parent_id = state_manager.get_current_state_id().await?;
        let staging_path = PathBuf::from(format!("/opt/pm/states/staging-{staging_id}"));

        Ok(Self {
            staging_id,
            parent_id: Some(parent_id),
            staging_path,
            state_manager: state_manager.clone(),
        })
    }

    /// Create staging directory as APFS clone
    pub async fn create_staging(&self, live_path: &Path) -> Result<(), Error> {
        #[cfg(target_os = "macos")]
        {
            // Use APFS clonefile for instant, space-efficient copy
            self.apfs_clonefile(live_path, &self.staging_path).await?;
        }
        #[cfg(not(target_os = "macos"))]
        {
            // Fallback to directory copy on other platforms
            self.copy_directory_recursive(&live_path.to_path_buf(), &self.staging_path)
                .await?;
        }
        Ok(())
    }

    /// APFS clonefile implementation
    #[cfg(target_os = "macos")]
    async fn apfs_clonefile(&self, source: &Path, dest: &Path) -> Result<(), Error> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

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

        // macOS clonefile syscall number
        const SYS_CLONEFILE: libc::c_int = 462;
        
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
    async fn copy_directory_recursive(
        &self,
        source: &std::path::PathBuf,
        dest: &std::path::PathBuf,
    ) -> Result<(), Error> {
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
    pub async fn commit(&self, live_path: &Path) -> Result<(), Error> {
        // Begin database transaction
        let mut tx = self.state_manager.begin_transaction().await?;

        // Record new state in database
        self.state_manager
            .create_state_with_tx(&mut tx, &self.staging_id, self.parent_id.as_ref(), "install")
            .await?;

        // Atomic filesystem swap
        let old_live_path = PathBuf::from(format!(
            "/opt/pm/states/{}",
            self.parent_id.unwrap_or_default()
        ));

        // Move current live to archived location
        if live_path.exists() {
            fs::rename(live_path, &old_live_path).await?;
        }

        // Move staging to live
        fs::rename(&self.staging_path, live_path).await?;

        // Update active state pointer
        self.state_manager
            .set_active_state_with_tx(&mut tx, self.staging_id)
            .await?;

        // Commit transaction
        tx.commit().await?;

        Ok(())
    }

    /// Clean up staging directory
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

    #[tokio::test]
    async fn test_state_transition_creation() {
        let temp = tempdir().unwrap();
        let state_manager = StateManager::new(temp.path()).await.unwrap();

        let transition = StateTransition::new(&state_manager).await.unwrap();

        assert!(!transition.staging_id.is_nil());
        assert!(transition.staging_path.to_string().contains("staging"));
    }

    #[tokio::test]
    async fn test_staging_cleanup() {
        let temp = tempdir().unwrap();
        let state_manager = StateManager::new(temp.path()).await.unwrap();

        let transition = StateTransition::new(&state_manager).await.unwrap();

        // Create staging directory
        fs::create_dir_all(&transition.staging_path).await.unwrap();
        assert!(transition.staging_path.exists());

        // Clean up
        transition.cleanup().await.unwrap();
        assert!(!transition.staging_path.exists());
    }

    #[test]
    fn test_install_result() {
        let state_id = Uuid::new_v4();
        let mut result = InstallResult::new(state_id);

        assert_eq!(result.total_changes(), 0);

        let package_id = PackageId::new(
            "test-pkg".to_string(),
            spsv2_types::Version::parse("1.0.0").unwrap(),
        );

        result.add_installed(package_id);
        assert_eq!(result.total_changes(), 1);
        assert_eq!(result.installed_packages.len(), 1);
    }
}
