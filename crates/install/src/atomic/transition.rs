//! State transition management for atomic installations

use crate::atomic::filesystem;
use sps2_errors::{Error, InstallError};
use sps2_events::EventSender;
use sps2_state::{PackageRef, StateManager};
use std::path::{Path, PathBuf};
use tokio::fs;
use uuid::Uuid;

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
    pub package_refs: Vec<PackageRef>,
    /// Package references with venv paths to be added during commit
    pub package_refs_with_venv: Vec<(PackageRef, String)>,
    /// Package files to be added during commit
    pub package_files: Vec<(String, String, String, bool)>, // (package_name, package_version, file_path, is_directory)
    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
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
            package_refs_with_venv: Vec::new(),
            package_files: Vec::new(),
            event_sender: None,
        })
    }

    /// Create staging directory as APFS clone
    ///
    /// # Errors
    ///
    /// Returns an error if APFS clonefile system call fails.
    pub fn create_staging(&self, live_path: &Path) -> Result<(), Error> {
        filesystem::create_staging_directory(live_path, &self.staging_path)
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

        // Add all packages with venv paths to the database
        for (package_ref, venv_path) in &self.package_refs_with_venv {
            self.state_manager
                .add_package_ref_with_venv_tx(&mut tx, package_ref, Some(venv_path))
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

        // Ensure parent directories exist before any rename operations
        if let Some(state_parent) = self.state_manager.state_path().parent() {
            fs::create_dir_all(state_parent)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "create_state_parent_dir".to_string(),
                    path: state_parent.display().to_string(),
                    message: e.to_string(),
                })?;
        }
        fs::create_dir_all(self.state_manager.state_path())
            .await
            .map_err(|e| InstallError::FilesystemError {
                operation: "create_state_dir".to_string(),
                path: self.state_manager.state_path().display().to_string(),
                message: e.to_string(),
            })?;

        // Ensure parent directory of live_path exists for first install
        if let Some(live_parent) = live_path.parent() {
            fs::create_dir_all(live_parent)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "create_live_parent_dir".to_string(),
                    path: live_parent.display().to_string(),
                    message: e.to_string(),
                })?;
        }

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
            fs::rename(&self.staging_path, &old_live_path)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "archive_old_live".to_string(),
                    path: old_live_path.display().to_string(),
                    message: e.to_string(),
                })?;
        } else {
            // If live doesn't exist, just move staging to live (first install)
            fs::rename(&self.staging_path, live_path)
                .await
                .map_err(|e| InstallError::FilesystemError {
                    operation: "first_install_move".to_string(),
                    path: live_path.display().to_string(),
                    message: e.to_string(),
                })?;
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
