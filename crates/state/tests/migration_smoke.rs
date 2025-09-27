use tempfile::TempDir;

#[tokio::test]
async fn migrations_apply_and_expose_core_tables() {
    let temp_dir = TempDir::new().expect("temp dir");
    let db_path = temp_dir.path().join("state.sqlite");

    let pool = sps2_state::create_pool(&db_path)
        .await
        .expect("create pool");
    sps2_state::run_migrations(&pool)
        .await
        .expect("run migrations");

    let mut conn = pool.acquire().await.expect("acquire connection");

    for table in [
        "states",
        "state_packages",
        "package_versions",
        "cas_objects",
        "package_files",
        "file_verification",
    ] {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")
                .bind(table)
                .fetch_optional(&mut *conn)
                .await
                .expect("check table existence");
        assert!(exists.is_some(), "expected table `{table}` to exist");
    }
}
