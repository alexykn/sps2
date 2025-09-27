//! tests/recovery.rs

use sps2_state::StateManager;
use sps2_state::TransactionData;
use sps2_state::PackageRef;
use sps2_types::Version;
use std::collections::HashMap;
use tempfile::TempDir;
use uuid::Uuid;

async fn mk_state() -> (TempDir, StateManager) {
    let td = TempDir::new().expect("tempdir");
    let mgr = StateManager::new(td.path()).await.expect("state new");
    (td, mgr)
}

#[tokio::test]
async fn test_recovers_from_prepared_state() {
    let (_td, mut state) = mk_state().await;
    let parent_id = state.get_current_state_id().await.unwrap();

    // 1. Prepare a transaction, which writes the journal in the `Prepared` state.
    let staging_id = Uuid::new_v4();
    let pkg_hash = sps2_hash::Hash::from_data(b"prepared-pkg").to_hex();
    let pid = sps2_resolver::PackageId::new("A".to_string(), Version::parse("1.0.0").unwrap());
    let pref = PackageRef {
        state_id: staging_id,
        package_id: pid.clone(),
        hash: pkg_hash.clone(),
        size: 1,
    };
    let td = TransactionData {
        package_refs: &[pref],
        file_references: &[],
        pending_file_hashes: &[],
    };
    let staging_slot = state.inactive_slot().await;
    let journal = state
        .prepare_transaction(&staging_id, &parent_id, staging_slot, "install", &td)
        .await
        .unwrap();

    // At this point, a crash happens. The journal file exists.
    // The live directory should still point to the parent state.
    let live_path = state.live_path().to_path_buf();
    assert!(!live_path.join("A-1.0.0").exists(), "Live dir should not be updated yet");

    // 2. Simulate an application restart by creating a new StateManager.
    // The new manager should automatically run recovery.
    let state_base_path = _td.path().to_path_buf();
    let new_state_manager = StateManager::new(&state_base_path).await.unwrap();

    // 3. Verify the outcome.
    // The new state should be active.
    let active_id = new_state_manager.get_current_state_id().await.unwrap();
    assert_eq!(active_id, staging_id);

    // The journal file should be gone.
    let journal_path = state_base_path.join("transaction.json");
    assert!(!tokio::fs::metadata(&journal_path).await.is_ok(), "Journal file should be cleared after recovery");

    // The live directory should have been swapped.
    // We can't easily check the content here without more setup, but we can check the slot state.
    let active_slot = new_state_manager.active_slot().await;
    assert_eq!(active_slot, journal.staging_slot);
    let new_slot_state = new_state_manager.slot_state(active_slot).await;
    assert_eq!(new_slot_state, Some(staging_id));
}

#[tokio::test]
async fn test_recovers_from_swapped_state() {
    let (_td, mut state) = mk_state().await;
    let parent_id = state.get_current_state_id().await.unwrap();

    // 1. Prepare a transaction.
    let staging_id = Uuid::new_v4();
    let td = TransactionData {
        package_refs: &[],
        file_references: &[],
        pending_file_hashes: &[],
    };
    let staging_slot = state.inactive_slot().await;
    let mut journal = state
        .prepare_transaction(&staging_id, &parent_id, staging_slot, "test", &td)
        .await
        .unwrap();

    // 2. Manually perform the swap and update the journal to the `Swapped` phase.
    {
        let mut slots = state.live_slots.lock().await; // Assuming live_slots is public for test
        slots
            .swap_to_live(journal.staging_slot, journal.new_state_id, journal.parent_state_id)
            .await
            .unwrap();
    }
    journal.phase = sps2_types::state::TransactionPhase::Swapped;
    state.write_journal(&journal).await.unwrap();

    // At this point, a crash happens. The filesystem is updated, but the DB is not.
    let db_active_id = state.get_current_state_id().await.unwrap();
    assert_eq!(db_active_id, parent_id, "DB active state should not be updated yet");

    // 3. Simulate an application restart.
    let state_base_path = _td.path().to_path_buf();
    let new_state_manager = StateManager::new(&state_base_path).await.unwrap();

    // 4. Verify the outcome.
    // The new state should be active in the DB.
    let active_id = new_state_manager.get_current_state_id().await.unwrap();
    assert_eq!(active_id, staging_id);

    // The journal file should be gone.
    let journal_path = state_base_path.join("transaction.json");
    assert!(!tokio::fs::metadata(&journal_path).await.is_ok(), "Journal file should be cleared after recovery");
}
