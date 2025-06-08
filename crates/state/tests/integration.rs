//! Integration tests for state management

mod venv_tracking;

#[cfg(test)]
mod tests {
    use sps2_state::*;
    use sps2_types::Version;
    use tempfile::tempdir;
    use uuid::Uuid;

    async fn setup_test_manager() -> (StateManager, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("state.sqlite");

        // Create required directories
        tokio::fs::create_dir_all(temp_dir.path().join("states"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(temp_dir.path().join("live"))
            .await
            .unwrap();

        // Add some initial content to the live directory so cloning works
        tokio::fs::write(
            temp_dir.path().join("live").join("initial.txt"),
            b"initial state",
        )
        .await
        .unwrap();

        // Setup database and create initial state manually
        let pool = create_pool(&db_path).await.unwrap();
        run_migrations(&pool).await.unwrap();

        // Create initial state
        let mut tx = pool.begin().await.unwrap();
        let initial_id = Uuid::new_v4();

        // Use raw SQL to avoid SQLX offline mode issues
        sqlx::query("INSERT INTO states (id, parent_id, created_at, operation, success) VALUES (?, NULL, ?, 'initial', 1)")
            .bind(initial_id.to_string())
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *tx)
            .await
            .unwrap();

        sqlx::query("INSERT INTO active_state (id, state_id, updated_at) VALUES (1, ?, ?)")
            .bind(initial_id.to_string())
            .bind(chrono::Utc::now().timestamp())
            .execute(&mut *tx)
            .await
            .unwrap();

        tx.commit().await.unwrap();

        // Create manager with the pool
        let (event_tx, _) = sps2_events::channel();
        let manager = StateManager::with_pool(
            pool,
            temp_dir.path().join("states"),
            temp_dir.path().join("live"),
            event_tx,
        );

        (manager, temp_dir)
    }

    #[tokio::test]
    async fn test_state_transitions() {
        let (manager, _temp_dir) = setup_test_manager().await;

        // Get initial state - there should be one from initialization
        let initial_state = manager.get_active_state().await.unwrap();

        // Begin transition
        let transition = manager.begin_transition("test install").await.unwrap();
        assert_ne!(transition.from, transition.to);

        // Commit with a package
        let pkg = models::PackageRef {
            state_id: transition.to,
            package_id: sps2_resolver::PackageId {
                name: "test-pkg".to_string(),
                version: Version::parse("1.0.0").unwrap(),
            },
            hash: "test-hash-1234".to_string(),
            size: 1024,
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
        let (manager, temp_dir) = setup_test_manager().await;
        let state_path = temp_dir.path().join("states");

        // Get initial state
        let initial_state = manager.get_active_state().await.unwrap();

        // Create state directory for initial state (required for rollback)
        tokio::fs::create_dir_all(state_path.join(initial_state.to_string()))
            .await
            .unwrap();

        // Make a change
        let transition = manager.begin_transition("test change").await.unwrap();
        let pkg = models::PackageRef {
            state_id: transition.to,
            package_id: sps2_resolver::PackageId {
                name: "temp-pkg".to_string(),
                version: Version::parse("1.0.0").unwrap(),
            },
            hash: "temp-hash-5678".to_string(),
            size: 2048,
        };
        manager
            .commit_transition(transition, vec![pkg], vec![])
            .await
            .unwrap();

        // Get state after change
        let changed_state = manager.get_active_state().await.unwrap();
        assert_ne!(initial_state, changed_state);

        // Rollback
        manager.rollback(None).await.unwrap();

        // Verify rollback succeeded by checking we're not on the changed state
        let current = manager.get_active_state().await.unwrap();
        assert_ne!(current, changed_state);
    }
}
