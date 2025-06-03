//! Rollback operations for atomic installations

use sps2_errors::{Error, InstallError};
use sps2_state::StateManager;
use std::path::Path;
use uuid::Uuid;

/// Rollback to a previous state
///
/// # Errors
///
/// Returns an error if the target state doesn't exist, filesystem swap fails,
/// or database update fails.
pub async fn rollback_to_state(
    state_manager: &StateManager,
    live_path: &Path,
    target_state_id: Uuid,
) -> Result<(), Error> {
    let target_path = state_manager.get_state_path(target_state_id)?;

    // Use true atomic swap to exchange target state with live directory
    sps2_root::atomic_swap(&target_path, live_path)
        .await
        .map_err(|e| InstallError::FilesystemError {
            operation: "rollback_atomic_swap".to_string(),
            path: target_path.display().to_string(),
            message: e.to_string(),
        })?;

    // Update active state in database
    state_manager.set_active_state(target_state_id).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use tokio::fs;

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
}
