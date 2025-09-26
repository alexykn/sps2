//! State transition management for atomic installations

use sps2_events::EventSender;
use sps2_hash::FileHashResult;
use sps2_platform::{core::PlatformContext, PlatformManager};
use sps2_state::{FileReference, PackageRef, StateManager};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagingMode {
    Cloned,
    Fresh,
}

#[derive(Debug)]
pub struct StagingCreation {
    pub mode: StagingMode,
    pub clone_attempted: bool,
    pub clone_error: Option<String>,
}

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
    /// How the staging directory was initialized
    pub staging_mode: StagingMode,
    /// Package references to be added during commit
    pub package_refs: Vec<PackageRef>,
    // Removed package_refs_with_venv - Python packages now handled like regular packages
    /// Package files to be added during commit (legacy)
    pub package_files: Vec<(String, String, String, bool)>, // (package_name, package_version, file_path, is_directory)
    /// File references for file-level storage
    pub file_references: Vec<(i64, FileReference)>, // (package_id, file_reference)
    /// Pending file hashes to be converted to file references after we have package IDs
    pub pending_file_hashes: Vec<(sps2_resolver::PackageId, Vec<FileHashResult>)>,
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
            staging_mode: StagingMode::Fresh,
            package_refs: Vec::new(),
            package_files: Vec::new(),
            file_references: Vec::new(),
            pending_file_hashes: Vec::new(),
            event_sender: None,
            operation,
        })
    }

    /// Create a platform context for filesystem operations
    fn create_platform_context(&self) -> (&'static sps2_platform::Platform, PlatformContext) {
        let platform = PlatformManager::instance().platform();
        let context = platform.create_context(None);
        (platform, context)
    }

    /// Clean up staging directory
    ///
    /// # Errors
    ///
    /// Returns an error if directory removal fails.
    pub async fn cleanup(&self, state_manager: &StateManager) -> Result<(), sps2_errors::Error> {
        let (platform, ctx) = self.create_platform_context();

        if platform.filesystem().exists(&ctx, &self.staging_path).await {
            // Check if staging directory can be safely removed
            if state_manager.can_remove_staging(&self.staging_id).await? {
                platform
                    .filesystem()
                    .remove_dir_all(&ctx, &self.staging_path)
                    .await
                    .map_err(|e| sps2_errors::InstallError::FilesystemError {
                        operation: "remove_dir_all".to_string(),
                        path: self.staging_path.display().to_string(),
                        message: e.to_string(),
                    })?;
            }
        }
        Ok(())
    }
}
