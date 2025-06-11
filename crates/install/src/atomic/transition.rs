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
    /// Operation type (install, uninstall, etc.)
    operation: String,
}

impl StateTransition {
    /// Create new state transition
    ///
    /// # Errors
    ///
    /// Returns an error if getting current state ID fails.
    pub async fn new(state_manager: &StateManager, operation: String) -> Result<Self, Error> {
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
            operation,
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
        // Get the current state ID before we start the transaction
        // This is what's actually in /opt/pm/live right now
        let actual_current_state_id = self.state_manager.get_current_state_id().await?;

        // Begin database transaction
        let mut tx = self.state_manager.begin_transaction().await?;

        // Record new state in database
        self.state_manager
            .create_state_with_tx(
                &mut tx,
                &self.staging_id,
                self.parent_id.as_ref(),
                &self.operation,
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
        // Use the actual current state ID we captured before the transaction
        let old_live_path = self
            .state_manager
            .state_path()
            .join(actual_current_state_id.to_string());

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

        // Debug: Check staging directory before swap
        if let Some(sender) = &self.event_sender {
            let _ = sender.send(sps2_events::Event::DebugLog {
                message: format!(
                    "Before swap - staging path exists: {}, live path exists: {}",
                    self.staging_path.exists(),
                    live_path.exists()
                ),
                context: std::collections::HashMap::new(),
            });

            // List staging directory contents
            if let Ok(mut entries) = tokio::fs::read_dir(&self.staging_path).await {
                let mut count = 0;
                while let Ok(Some(entry)) = entries.next_entry().await {
                    count += 1;
                    let _ = sender.send(sps2_events::Event::DebugLog {
                        message: format!(
                            "Staging contains: {}",
                            entry.file_name().to_string_lossy()
                        ),
                        context: std::collections::HashMap::new(),
                    });
                }
                let _ = sender.send(sps2_events::Event::DebugLog {
                    message: format!("Total items in staging: {}", count),
                    context: std::collections::HashMap::new(),
                });
            }
        }

        // True atomic swap using sps2_root::atomic_swap
        // This uses macOS renamex_np with RENAME_SWAP for single OS-level atomic operation
        if live_path.exists() {
            // Debug: Log paths before swap
            if let Some(sender) = &self.event_sender {
                let _ = sender.send(sps2_events::Event::DebugLog {
                    message: format!(
                        "Attempting atomic swap: staging={}, live={}",
                        self.staging_path.display(),
                        live_path.display()
                    ),
                    context: std::collections::HashMap::new(),
                });
            }

            // Use atomic swap to exchange staging and live directories
            match sps2_root::atomic_swap(&self.staging_path, live_path).await {
                Ok(()) => {
                    if let Some(sender) = &self.event_sender {
                        let _ = sender.send(sps2_events::Event::DebugLog {
                            message: "Atomic swap successful".to_string(),
                            context: std::collections::HashMap::new(),
                        });
                    }
                }
                Err(e) => {
                    if let Some(sender) = &self.event_sender {
                        let _ = sender.send(sps2_events::Event::DebugLog {
                            message: format!("Atomic swap FAILED: {}", e),
                            context: std::collections::HashMap::new(),
                        });
                    }
                    return Err(InstallError::FilesystemError {
                        operation: "atomic_swap".to_string(),
                        path: live_path.display().to_string(),
                        message: e.to_string(),
                    }
                    .into());
                }
            }

            // Move the old live directory (now in staging_path) to archived location
            if !old_live_path.exists() {
                // Archive normally if state doesn't exist
                if let Some(sender) = &self.event_sender {
                    let _ = sender.send(sps2_events::Event::DebugLog {
                        message: format!(
                            "Archiving old state {} to {}",
                            actual_current_state_id,
                            old_live_path.display()
                        ),
                        context: std::collections::HashMap::new(),
                    });
                }

                fs::rename(&self.staging_path, &old_live_path)
                    .await
                    .map_err(|e| InstallError::FilesystemError {
                        operation: "archive_old_live".to_string(),
                        path: old_live_path.display().to_string(),
                        message: e.to_string(),
                    })?;
            } else {
                // State already exists - archive with timestamp to avoid collision
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
                let alternative_path = self
                    .state_manager
                    .state_path()
                    .join(format!("{}-{}", actual_current_state_id, timestamp));

                if let Some(sender) = &self.event_sender {
                    let _ = sender.send(sps2_events::Event::DebugLog {
                        message: format!(
                            "State {} already archived, using alternative path: {}",
                            actual_current_state_id,
                            alternative_path.display()
                        ),
                        context: std::collections::HashMap::new(),
                    });
                }

                fs::rename(&self.staging_path, &alternative_path)
                    .await
                    .map_err(|e| InstallError::FilesystemError {
                        operation: "archive_old_live_alternative".to_string(),
                        path: alternative_path.display().to_string(),
                        message: e.to_string(),
                    })?;
            }
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
