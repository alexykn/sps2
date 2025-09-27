use sps2_hash::Hash;
use tempfile::TempDir;

#[tokio::test]
async fn add_and_fetch_package_round_trip() {
    let temp_dir = TempDir::new().expect("tempdir");
    let db_path = temp_dir.path().join("state.sqlite");

    let pool = sps2_state::create_pool(&db_path)
        .await
        .expect("create pool");
    sps2_state::run_migrations(&pool)
        .await
        .expect("run migrations");

    let state_id = uuid::Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin tx");
    sps2_state::queries::create_state(&mut tx, &state_id, None, "install")
        .await
        .expect("create state");
    sps2_state::queries::set_active_state(&mut tx, &state_id)
        .await
        .expect("set active state");
    let pkg_row =
        sps2_state::queries::add_package(&mut tx, &state_id, "hello", "1.0.0", "store-hash", 42)
            .await
            .expect("add package");
    tx.commit().await.expect("commit");

    let mut tx = pool.begin().await.expect("begin tx2");
    let packages = sps2_state::queries::get_state_packages(&mut tx, &state_id)
        .await
        .expect("get packages");
    assert_eq!(packages.len(), 1);
    let pkg = &packages[0];
    assert_eq!(pkg.id, pkg_row);
    assert_eq!(pkg.name, "hello");
    assert_eq!(pkg.version, "1.0.0");
    assert_eq!(pkg.hash, "store-hash");
    assert_eq!(pkg.size, 42);
}

#[tokio::test]
async fn add_file_object_and_entry_round_trip() {
    use sps2_state::file_queries_runtime as files;
    use sps2_state::queries;

    let temp_dir = TempDir::new().expect("tempdir");
    let db_path = temp_dir.path().join("state.sqlite");

    let pool = sps2_state::create_pool(&db_path)
        .await
        .expect("create pool");
    sps2_state::run_migrations(&pool)
        .await
        .expect("run migrations");

    let state_id = uuid::Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin tx");
    queries::create_state(&mut tx, &state_id, None, "install")
        .await
        .expect("create state");
    queries::set_active_state(&mut tx, &state_id)
        .await
        .expect("set active state");
    let pkg_row = queries::add_package(&mut tx, &state_id, "pkg", "1.0.0", "store-hash", 10)
        .await
        .expect("add package");

    let file_hash = Hash::from_data(b"hello-file");
    let metadata = sps2_state::file_models::FileMetadata {
        size: 5,
        permissions: 0o755,
        uid: 0,
        gid: 0,
        mtime: None,
        is_executable: true,
        is_symlink: false,
        symlink_target: None,
    };
    files::add_file_object(&mut tx, &file_hash, &metadata)
        .await
        .expect("add file object");

    let file_ref = sps2_state::file_models::FileReference {
        package_id: pkg_row,
        relative_path: "bin/hello".to_string(),
        hash: file_hash.clone(),
        metadata: metadata.clone(),
    };
    files::add_package_file_entry(&mut tx, pkg_row, &file_ref)
        .await
        .expect("add file entry");
    tx.commit().await.expect("commit");

    let mut tx = pool.begin().await.expect("begin tx2");
    let entries = files::get_package_file_entries(&mut tx, pkg_row)
        .await
        .expect("get file entries");
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.relative_path, "bin/hello");
    assert_eq!(entry.file_hash, file_hash.to_hex());
}
