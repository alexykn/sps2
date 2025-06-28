//! State transition management for atomic installations

use sps2_events::EventSender;
use sps2_state::{FileReference, PackageRef, StateManager};
use std::path::PathBuf;
use uuid::Uuid;

/// State transition for atomic operations
///
/// This is now a simple data container that holds information about
/// a pending state transition. The actual commit logic is handled
/// by the StateManager using two-phase commit.
pub struct StateTransition {
    /// Staging state ID
    pub staging_id: Uuid,
    /// Parent state ID  
    pub parent_id: Option<Uuid>,
    /// Staging directory path
    pub staging_path: PathBuf,
    /// Package references to be added during commit
    pub package_refs: Vec<PackageRef>,
    /// Package references with venv paths to be added during commit
    pub package_refs_with_venv: Vec<(PackageRef, String)>,
    /// Package files to be added during commit (legacy)
    pub package_files: Vec<(String, String, String, bool)>, // (package_name, package_version, file_path, is_directory)
    /// File references for file-level storage
    pub file_references: Vec<(i64, FileReference)>, // (package_id, file_reference)
    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
    /// Operation type (install, uninstall, etc.)
    pub operation: String,
}

impl StateTransition {
    /// Create new state transition
    ///
    /// # Errors
    ///
    /// Returns an error if getting current state ID fails.
    pub async fn new(
        state_manager: &StateManager,
        operation: String,
    ) -> Result<Self, sps2_errors::Error> {
        let staging_id = Uuid::new_v4();
        let parent_id = state_manager.get_current_state_id().await?;
        let staging_path = state_manager
            .state_path()
            .join(format!("staging-{staging_id}"));

        Ok(Self {
            staging_id,
            parent_id: Some(parent_id),
            staging_path,
            package_refs: Vec::new(),
            package_refs_with_venv: Vec::new(),
            package_files: Vec::new(),
            file_references: Vec::new(),
            event_sender: None,
            operation,
        })
    }

    /// Clean up staging directory
    ///
    /// # Errors
    ///
    /// Returns an error if directory removal fails.
    pub async fn cleanup(&self) -> Result<(), sps2_errors::Error> {
        if sps2_root::exists(&self.staging_path).await {
            sps2_root::remove_dir_all(&self.staging_path).await?;
        }
        Ok(())
    }
}
