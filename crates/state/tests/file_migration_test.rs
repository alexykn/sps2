//! Tests for file-level storage migration

use sps2_hash::Hash;
use sps2_state::{create_pool, file_migration::*, file_models::*, queries::*, run_migrations};
use sqlx::query;
use tempfile::TempDir;

#[tokio::test]
async fn test_file_object_storage() {
    // Create temporary database
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let pool = create_pool(&db_path).await.unwrap();
    run_migrations(&pool).await.unwrap();

    // Test adding a file object
    let hash = Hash::from_data(b"test content");
    let metadata = FileMetadata::regular_file(1024, 0o644);

    let mut tx = pool.begin().await.unwrap();
    let result = add_file_object(&mut tx, &hash, &metadata).await.unwrap();
    assert!(!result.was_duplicate);
    assert_eq!(result.ref_count, 1);

    // Test deduplication
    let result2 = add_file_object(&mut tx, &hash, &metadata).await.unwrap();
    tx.commit().await.unwrap();
    assert!(result2.was_duplicate);
    assert_eq!(result2.ref_count, 2);
    assert_eq!(result2.space_saved, 1024);
}

#[tokio::test]
async fn test_package_file_entries() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let pool = create_pool(&db_path).await.unwrap();
    run_migrations(&pool).await.unwrap();

    // First create a test package
    let mut tx = pool.begin().await.unwrap();

    // Create a state
    let state_id = uuid::Uuid::new_v4();
    query("INSERT INTO states (id, created_at, operation, success) VALUES (?, ?, ?, ?)")
        .bind(state_id.to_string())
        .bind(chrono::Utc::now().timestamp())
        .bind("test")
        .bind(true)
        .execute(&mut *tx)
        .await
        .unwrap();

    // Create a package
    let package_id = query(
        r#"
        INSERT INTO packages (state_id, name, version, hash, size, installed_at)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(state_id.to_string())
    .bind("test-package")
    .bind("1.0.0")
    .bind("testhash")
    .bind(1024i64)
    .bind(chrono::Utc::now().timestamp())
    .execute(&mut *tx)
    .await
    .unwrap()
    .last_insert_rowid();

    // Add file objects and entries
    let file1_hash = Hash::from_data(b"file1 content");
    let file1_metadata = FileMetadata::regular_file(100, 0o755);
    add_file_object(&mut tx, &file1_hash, &file1_metadata)
        .await
        .unwrap();

    let file_ref = FileReference {
        package_id,
        relative_path: "bin/tool".to_string(),
        hash: file1_hash,
        metadata: file1_metadata,
    };

    let entry_id = add_package_file_entry(&mut tx, package_id, &file_ref)
        .await
        .unwrap();
    assert!(entry_id > 0);

    // Verify we can retrieve the files
    let files = get_package_file_entries(&mut tx, package_id).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].relative_path, "bin/tool");

    tx.commit().await.unwrap();
}

#[tokio::test]
async fn test_file_mtime_tracker() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let pool = create_pool(&db_path).await.unwrap();
    run_migrations(&pool).await.unwrap();

    let hash = Hash::from_data(b"test file");
    let path = "/opt/pm/live/bin/test";

    // Add file object first
    let metadata = FileMetadata::regular_file(1024, 0o755);
    let mut tx = pool.begin().await.unwrap();
    add_file_object(&mut tx, &hash, &metadata).await.unwrap();

    // Update mtime tracker
    let current_mtime = chrono::Utc::now().timestamp();
    update_file_mtime(&mut tx, path, current_mtime)
        .await
        .unwrap();

    // Retrieve mtime tracker
    let tracker = get_file_mtime(&mut tx, path).await.unwrap();
    tx.commit().await.unwrap();
    assert!(tracker.is_some());

    let entry = tracker.unwrap();
    assert_eq!(entry.file_path, path);
    assert_eq!(entry.last_verified_mtime, current_mtime);
}

#[tokio::test]
async fn test_storage_stats() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let pool = create_pool(&db_path).await.unwrap();
    run_migrations(&pool).await.unwrap();

    // Create test data
    let mut tx = pool.begin().await.unwrap();

    // Create state and package
    let state_id = uuid::Uuid::new_v4();
    query("INSERT INTO states (id, created_at, operation, success) VALUES (?, ?, ?, ?)")
        .bind(state_id.to_string())
        .bind(chrono::Utc::now().timestamp())
        .bind("test")
        .bind(true)
        .execute(&mut *tx)
        .await
        .unwrap();

    let package_id = query(
        r#"
        INSERT INTO packages (state_id, name, version, hash, size, installed_at)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(state_id.to_string())
    .bind("test-package")
    .bind("1.0.0")
    .bind("testhash")
    .bind(1024i64)
    .bind(chrono::Utc::now().timestamp())
    .execute(&mut *tx)
    .await
    .unwrap()
    .last_insert_rowid();

    // Add some files with deduplication
    let shared_hash = Hash::from_data(b"shared content");
    let shared_metadata = FileMetadata::regular_file(1000, 0o644);
    add_file_object(&mut tx, &shared_hash, &shared_metadata)
        .await
        .unwrap();

    // Add to package multiple times (simulating deduplication)
    for i in 0..3 {
        let file_ref = FileReference {
            package_id,
            relative_path: format!("lib/shared{i}.so"),
            hash: shared_hash.clone(),
            metadata: shared_metadata.clone(),
        };
        add_package_file_entry(&mut tx, package_id, &file_ref)
            .await
            .unwrap();
    }

    tx.commit().await.unwrap();

    // Get stats - this function doesn't exist in runtime queries yet
    // let stats = get_file_storage_stats(&pool).await.unwrap();
    // assert_eq!(stats.total_files, 3);
    // assert_eq!(stats.unique_files, 1);
    // assert_eq!(stats.total_size, 3000); // 3 files * 1000 bytes
    // assert_eq!(stats.deduplicated_size, 1000); // Only 1 unique file
}

#[tokio::test]
async fn test_migration_helpers() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let pool = create_pool(&db_path).await.unwrap();
    run_migrations(&pool).await.unwrap();

    // Create test package
    let mut tx = pool.begin().await.unwrap();

    let state_id = uuid::Uuid::new_v4();
    query("INSERT INTO states (id, created_at, operation, success) VALUES (?, ?, ?, ?)")
        .bind(state_id.to_string())
        .bind(chrono::Utc::now().timestamp())
        .bind("test")
        .bind(true)
        .execute(&mut *tx)
        .await
        .unwrap();

    let package = sps2_state::models::Package {
        id: 1,
        state_id: state_id.to_string(),
        name: "test-package".to_string(),
        version: "1.0.0".to_string(),
        hash: "oldhash".to_string(),
        size: 1024,
        installed_at: chrono::Utc::now().timestamp(),
        venv_path: None,
    };

    query(
        r#"
        INSERT INTO packages (id, state_id, name, version, hash, size, installed_at, has_file_hashes)
        VALUES (?, ?, ?, ?, ?, ?, ?, 0)
        "#
    )
    .bind(package.id)
    .bind(&package.state_id)
    .bind(&package.name)
    .bind(&package.version)
    .bind(&package.hash)
    .bind(package.size)
    .bind(package.installed_at)
    .execute(&mut *tx)
    .await
    .unwrap();

    // Check migration status
    assert!(!is_package_migrated(&mut tx, package.id).await.unwrap());

    // Get packages needing migration
    let packages = get_packages_needing_migration(&mut tx, 10).await.unwrap();
    assert_eq!(packages.len(), 1);

    // Migrate package
    let status = migrate_package_placeholder(&mut tx, &package)
        .await
        .unwrap();
    assert_eq!(status.package_id, package.id);

    // Check migration status again
    assert!(is_package_migrated(&mut tx, package.id).await.unwrap());

    // Get migration stats
    let stats = get_migration_stats(&mut tx).await.unwrap();
    assert_eq!(stats.total_packages, 1);
    assert_eq!(stats.migrated_packages, 1);

    tx.commit().await.unwrap();
}
