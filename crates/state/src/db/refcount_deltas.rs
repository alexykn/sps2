use sps2_errors::{Error, StateError};
use sps2_types::StateId;
use sqlx::{query, Sqlite, Transaction};

/// Ensure CAS rows exist for archives referenced by the target state.
///
/// # Errors
///
/// Returns an error if the database operation fails.
async fn ensure_archive_cas_rows(
    tx: &mut Transaction<'_, Sqlite>,
    to_state: &StateId,
) -> Result<(), Error> {
    let to_str = to_state.to_string();

    query(
        r#"
        INSERT OR IGNORE INTO cas_objects (hash, kind, size_bytes, created_at, ref_count)
        SELECT DISTINCT pv.store_hash, 'archive', pv.size_bytes, strftime('%s','now'), 0
        FROM state_packages sp
        JOIN package_versions pv ON pv.id = sp.package_version_id
        WHERE sp.state_id = ?1 AND pv.store_hash IS NOT NULL
        "#,
    )
    .bind(&to_str)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("ensure archive cas rows failed: {e}"),
    })?;

    Ok(())
}

/// Calculate archive refcount increases for new hashes.
///
/// # Errors
///
/// Returns an error if the database operation fails.
async fn calculate_archive_increases(
    tx: &mut Transaction<'_, Sqlite>,
    from_state: Option<&StateId>,
    to_state: &StateId,
) -> Result<u64, Error> {
    let to_str = to_state.to_string();

    Ok(if let Some(from) = from_state {
        let from_str = from.to_string();
        query(
            r#"
            WITH
              to_hashes AS (
                SELECT DISTINCT pv.store_hash AS hash
                FROM state_packages sp
                JOIN package_versions pv ON pv.id = sp.package_version_id
                WHERE sp.state_id = ?1
              ),
              from_hashes AS (
                SELECT DISTINCT pv.store_hash AS hash
                FROM state_packages sp
                JOIN package_versions pv ON pv.id = sp.package_version_id
                WHERE sp.state_id = ?2
              ),
              new_hashes AS (
                SELECT hash FROM to_hashes
                EXCEPT
                SELECT hash FROM from_hashes
              )
            UPDATE cas_objects
               SET ref_count = ref_count + 1
            WHERE kind = 'archive' AND hash IN (SELECT hash FROM new_hashes)
            "#,
        )
        .bind(&to_str)
        .bind(&from_str)
        .execute(&mut **tx)
        .await?
        .rows_affected()
    } else {
        query(
            r#"
            UPDATE cas_objects
               SET ref_count = ref_count + 1
            WHERE kind = 'archive' AND hash IN (
              SELECT DISTINCT pv.store_hash
              FROM state_packages sp
              JOIN package_versions pv ON pv.id = sp.package_version_id
              WHERE sp.state_id = ?1
            )
            "#,
        )
        .bind(&to_str)
        .execute(&mut **tx)
        .await?
        .rows_affected()
    })
}

/// Calculate archive refcount decreases for removed hashes.
///
/// # Errors
///
/// Returns an error if the database operation fails.
async fn calculate_archive_decreases(
    tx: &mut Transaction<'_, Sqlite>,
    from_state: Option<&StateId>,
    to_state: &StateId,
) -> Result<u64, Error> {
    let to_str = to_state.to_string();

    Ok(if let Some(from) = from_state {
        let from_str = from.to_string();
        query(
            r#"
            WITH
              to_hashes AS (
                SELECT DISTINCT pv.store_hash AS hash
                FROM state_packages sp
                JOIN package_versions pv ON pv.id = sp.package_version_id
                WHERE sp.state_id = ?1
              ),
              from_hashes AS (
                SELECT DISTINCT pv.store_hash AS hash
                FROM state_packages sp
                JOIN package_versions pv ON pv.id = sp.package_version_id
                WHERE sp.state_id = ?2
              ),
              removed_hashes AS (
                SELECT hash FROM from_hashes
                EXCEPT
                SELECT hash FROM to_hashes
              )
            UPDATE cas_objects
               SET ref_count = ref_count - 1
            WHERE kind = 'archive' AND hash IN (SELECT hash FROM removed_hashes)
            "#,
        )
        .bind(&to_str)
        .bind(&from_str)
        .execute(&mut **tx)
        .await?
        .rows_affected()
    } else {
        0
    })
}

/// Apply archive refcount deltas when transitioning from `from_state` -> `to_state`.
///
/// # Errors
///
/// Returns an error if the database operations fail.
pub async fn apply_archive_refcount_deltas(
    tx: &mut Transaction<'_, Sqlite>,
    from_state: Option<&StateId>,
    to_state: &StateId,
) -> Result<(u64, u64), Error> {
    // Ensure CAS rows exist for archives referenced by the target state.
    ensure_archive_cas_rows(tx, to_state).await?;

    let inc_rows = calculate_archive_increases(tx, from_state, to_state).await?;
    let dec_rows = calculate_archive_decreases(tx, from_state, to_state).await?;

    Ok((inc_rows, dec_rows))
}

/// Ensure verification rows exist for file hashes.
///
/// # Errors
///
/// This function does not return errors as it uses `.ok()` to ignore them.
async fn ensure_file_verification_rows(tx: &mut Transaction<'_, Sqlite>, to_state: &StateId) {
    let to_str = to_state.to_string();

    query(
        r#"
        INSERT OR IGNORE INTO file_verification (file_hash, status, attempts, last_checked_at, last_error)
        SELECT DISTINCT pf.file_hash, 'pending', 0, NULL, NULL
        FROM state_packages sp
        JOIN package_files pf ON pf.package_version_id = sp.package_version_id
        WHERE sp.state_id = ?1
        "#,
    )
    .bind(&to_str)
    .execute(&mut **tx)
    .await
    .ok();
}

/// Calculate file refcount increases for new hashes.
///
/// # Errors
///
/// Returns an error if the database operation fails.
async fn calculate_file_increases(
    tx: &mut Transaction<'_, Sqlite>,
    from_state: Option<&StateId>,
    to_state: &StateId,
) -> Result<u64, Error> {
    let to_str = to_state.to_string();

    Ok(if let Some(from) = from_state {
        let from_str = from.to_string();
        query(
            r#"
            WITH
              to_hashes AS (
                SELECT DISTINCT pf.file_hash AS hash
                FROM state_packages sp
                JOIN package_files pf ON pf.package_version_id = sp.package_version_id
                WHERE sp.state_id = ?1
              ),
              from_hashes AS (
                SELECT DISTINCT pf.file_hash AS hash
                FROM state_packages sp
                JOIN package_files pf ON pf.package_version_id = sp.package_version_id
                WHERE sp.state_id = ?2
              ),
              new_hashes AS (
                SELECT hash FROM to_hashes
                EXCEPT
                SELECT hash FROM from_hashes
              )
            UPDATE cas_objects
               SET ref_count = ref_count + 1,
                   last_seen_at = strftime('%s','now')
            WHERE kind = 'file' AND hash IN (SELECT hash FROM new_hashes)
            "#,
        )
        .bind(&to_str)
        .bind(&from_str)
        .execute(&mut **tx)
        .await?
        .rows_affected()
    } else {
        query(
            r#"
            UPDATE cas_objects
               SET ref_count = ref_count + 1,
                   last_seen_at = strftime('%s','now')
            WHERE kind = 'file' AND hash IN (
              SELECT DISTINCT pf.file_hash
              FROM state_packages sp
              JOIN package_files pf ON pf.package_version_id = sp.package_version_id
              WHERE sp.state_id = ?1
            )
            "#,
        )
        .bind(&to_str)
        .execute(&mut **tx)
        .await?
        .rows_affected()
    })
}

/// Calculate file refcount decreases for removed hashes.
///
/// # Errors
///
/// Returns an error if the database operation fails.
async fn calculate_file_decreases(
    tx: &mut Transaction<'_, Sqlite>,
    from_state: Option<&StateId>,
    to_state: &StateId,
) -> Result<u64, Error> {
    let to_str = to_state.to_string();

    Ok(if let Some(from) = from_state {
        let from_str = from.to_string();
        query(
            r#"
            WITH
              to_hashes AS (
                SELECT DISTINCT pf.file_hash AS hash
                FROM state_packages sp
                JOIN package_files pf ON pf.package_version_id = sp.package_version_id
                WHERE sp.state_id = ?1
              ),
              from_hashes AS (
                SELECT DISTINCT pf.file_hash AS hash
                FROM state_packages sp
                JOIN package_files pf ON pf.package_version_id = sp.package_version_id
                WHERE sp.state_id = ?2
              ),
              removed_hashes AS (
                SELECT hash FROM from_hashes
                EXCEPT
                SELECT hash FROM to_hashes
              )
            UPDATE cas_objects
               SET ref_count = ref_count - 1
            WHERE kind = 'file' AND hash IN (SELECT hash FROM removed_hashes)
            "#,
        )
        .bind(&to_str)
        .bind(&from_str)
        .execute(&mut **tx)
        .await?
        .rows_affected()
    } else {
        0
    })
}

/// Apply file-object refcount deltas when transitioning from `from_state` -> `to_state`.
///
/// # Errors
///
/// Returns an error if the database operations fail.
pub async fn apply_file_refcount_deltas(
    tx: &mut Transaction<'_, Sqlite>,
    from_state: Option<&StateId>,
    to_state: &StateId,
) -> Result<(u64, u64), Error> {
    let inc_rows = calculate_file_increases(tx, from_state, to_state).await?;
    ensure_file_verification_rows(tx, to_state).await;
    let dec_rows = calculate_file_decreases(tx, from_state, to_state).await?;

    Ok((inc_rows, dec_rows))
}

/// Apply both archive and file refcount deltas.
///
/// # Errors
///
/// Returns an error if either the archive or file refcount operations fail.
pub async fn apply_all_refcount_deltas(
    tx: &mut Transaction<'_, Sqlite>,
    from_state: Option<&StateId>,
    to_state: &StateId,
) -> Result<((u64, u64), (u64, u64)), Error> {
    let arch = apply_archive_refcount_deltas(tx, from_state, to_state).await?;
    let files = apply_file_refcount_deltas(tx, from_state, to_state).await?;
    Ok((arch, files))
}
