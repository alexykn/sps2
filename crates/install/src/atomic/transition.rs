//! State transition management for atomic installations

use sps2_events::EventSender;
use sps2_hash::FileHashResult;
use sps2_state::{FileReference, PackageRef, StateManager};
use sps2_types::state::SlotId;
use std::path::PathBuf;
use uuid::Uuid;

/// State transition for atomic operations
///
/// This is now a simple data container that holds information about
/// a pending state transition. The actual commit logic is handled
/// by the `StateManager` using two-phase commit.
pub struct StateTransition {
    /// Staging state ID
    pub staging_id: Uuid,
    /// Parent state ID
    pub parent_id: Option<Uuid>,
    /// Slot that will hold the prepared state
    pub staging_slot: SlotId,
    /// Filesystem path to the staging slot
    pub slot_path: PathBuf,
    /// Package references to be added during commit
    pub package_refs: Vec<PackageRef>,
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
        let parent_id = Some(state_manager.get_current_state_id().await?);
        let staging_slot = state_manager.inactive_slot().await;
        let slot_path = state_manager.ensure_slot_dir(staging_slot).await?;

        Ok(Self {
            staging_id,
            parent_id,
            staging_slot,
            slot_path,
            package_refs: Vec::new(),
            file_references: Vec::new(),
            pending_file_hashes: Vec::new(),
            event_sender: None,
            operation,
        })
    }
}
