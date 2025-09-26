use sps2_errors::Error;
use sps2_state::StateManager;

/// Synchronize store and file-object refcounts to match the active state.
///
/// Returns the number of updated store rows and file rows respectively.
pub async fn sync_refcounts_to_active_state(state: &StateManager) -> Result<(usize, usize), Error> {
    let state_id = state.get_active_state().await?;
    state.sync_refcounts_to_state(&state_id).await
}
