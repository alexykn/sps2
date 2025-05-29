//! Integration tests for state management

#[cfg(test)]
mod tests {
    use spsv2_events::channel;
    use spsv2_hash::Hash;
    use spsv2_state::*;
    use spsv2_types::Version;
    use tempfile::tempdir;
    use uuid::Uuid;

    async fn setup_test_db() -> (Pool<sqlx::Sqlite>, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let pool = create_pool(&db_path).await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Initialize with a base state
        let mut tx = pool.begin().await.unwrap();
        let base_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO states (id, parent_id, created_at, operation, success) VALUES (?, NULL, ?, 'initial', 1)",
            base_id.to_string(),
            chrono::Utc::now().timestamp()
        )
        .execute(&mut *tx)
        .await
        .unwrap();

        sqlx::query!(
            "INSERT INTO active_state (id, state_id, updated_at) VALUES (1, ?, ?)",
            base_id.to_string(),
            chrono::Utc::now().timestamp()
        )
        .execute(&mut *tx)
        .await
        .unwrap();

        tx.commit().await.unwrap();

        (pool, temp_dir)
    }

    #[tokio::test]
    async fn test_state_transitions() {
        let (pool, temp_dir) = setup_test_db().await;
        let (tx, _rx) = channel();

        let state_path = temp_dir.path().join("states");
        let live_path = temp_dir.path().join("live");
        tokio::fs::create_dir_all(&state_path).await.unwrap();
        tokio::fs::create_dir_all(&live_path).await.unwrap();

        let manager = StateManager::new(pool, state_path, live_path, tx);

        // Get initial state
        let initial_state = manager.get_active_state().await.unwrap();

        // Begin transition
        let transition = manager.begin_transition("test install").await.unwrap();
        assert_ne!(transition.from, transition.to);

        // Commit with a package
        let pkg = models::PackageRef {
            name: "test-pkg".to_string(),
            version: Version::parse("1.0.0").unwrap(),
            hash: Hash::hash(b"test"),
            size: 1000,
        };

        manager
            .commit_transition(transition, vec![pkg], vec![])
            .await
            .unwrap();

        // Verify new state is active
        let new_state = manager.get_active_state().await.unwrap();
        assert_ne!(initial_state, new_state);

        // Verify package is installed
        let packages = manager.get_installed_packages().await.unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "test-pkg");
    }

    #[tokio::test]
    async fn test_rollback() {
        let (pool, temp_dir) = setup_test_db().await;
        let (tx, _rx) = channel();

        let state_path = temp_dir.path().join("states");
        let live_path = temp_dir.path().join("live");
        tokio::fs::create_dir_all(&state_path).await.unwrap();
        tokio::fs::create_dir_all(&live_path).await.unwrap();

        let manager = StateManager::new(pool, state_path.clone(), live_path, tx);

        // Get initial state
        let initial_state = manager.get_active_state().await.unwrap();

        // Create state directory for initial state
        tokio::fs::create_dir_all(state_path.join(initial_state.to_string()))
            .await
            .unwrap();

        // Make a change
        let transition = manager.begin_transition("test change").await.unwrap();
        let pkg = models::PackageRef {
            name: "temp-pkg".to_string(),
            version: Version::parse("1.0.0").unwrap(),
            hash: Hash::hash(b"temp"),
            size: 500,
        };
        manager
            .commit_transition(transition, vec![pkg], vec![])
            .await
            .unwrap();

        // Rollback
        manager.rollback(None).await.unwrap();

        // Verify we're back to initial state
        let current = manager.get_active_state().await.unwrap();
        assert_eq!(current, initial_state);
    }
}
