//! Rollback operations for atomic installations

use sps2_errors::Error;
use sps2_state::StateManager;
use sps2_types::state::{TransactionJournal, TransactionPhase};
use std::path::Path;
use uuid::Uuid;

/// Rollback to a previous state using 2PC
///
/// # Errors
///
/// Returns an error if the target state doesn't exist, journal write fails,
/// filesystem swap fails, or database update fails.
pub async fn rollback_to_state(
    state_manager: &StateManager,
    _live_path: &Path,
    target_state_id: Uuid,
) -> Result<(), Error> {
    // Get the current state ID before we start the rollback
    let current_state_id = state_manager.get_current_state_id().await?;

    // Get the target state path
    let target_path = state_manager.get_state_path(target_state_id)?;

    // Phase 1: Create and write the journal for rollback operation
    // We're using the target_path as the "staging" path since it's what we're swapping in
    let journal = TransactionJournal {
        new_state_id: target_state_id,
        parent_state_id: current_state_id, // The "parent" is what we are rolling back from
        staging_path: target_path,         // The "staging" path is the state we're rolling back to
        phase: TransactionPhase::Prepared,
        operation: "rollback".to_string(),
    };

    // Write the journal to disk - this is our commit point
    state_manager.write_journal(&journal).await?;

    // Phase 2: Execute the swap and finalize. The logic is identical to a normal install
    state_manager
        .execute_filesystem_swap_and_finalize(journal)
        .await?;

    Ok(())
}
