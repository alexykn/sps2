//! File-level queries for CAS (schema v2)

use crate::file_models::{
    DeduplicationResult, FileMTimeTracker, FileMetadata, FileObject, FileReference,
    PackageFileEntry,
};
use sps2_errors::{Error, StateError};
use sps2_hash::Hash;
use sqlx::{query, Row, Sqlite, Transaction};
use std::collections::HashMap;

/// Insert or increment a file object entry.
///
/// # Errors
///
/// Returns an error if the database operations fail.
pub async fn add_file_object(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &Hash,
    metadata: &FileMetadata,
) -> Result<DeduplicationResult, Error> {
    let hash_str = hash.to_hex();
    let now = chrono::Utc::now().timestamp();

    let existing = query("SELECT ref_count FROM cas_objects WHERE hash = ?1 AND kind = 'file'")
        .bind(&hash_str)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to check file object: {e}"),
        })?;

    if let Some(row) = existing {
        let current: i64 = row.get("ref_count");
        query("UPDATE cas_objects SET last_seen_at = ?2 WHERE hash = ?1 AND kind = 'file'")
            .bind(&hash_str)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(|e| StateError::DatabaseError {
                message: format!("failed to update file metadata: {e}"),
            })?;

        ensure_file_verification_row(tx, &hash_str).await?;
        return Ok(DeduplicationResult {
            hash: hash.clone(),
            was_duplicate: true,
            ref_count: current,
            space_saved: metadata.size,
        });
    }

    query(
        r#"
        INSERT INTO cas_objects (
            hash, kind, size_bytes, created_at, ref_count,
            is_executable, is_symlink, symlink_target,
            last_seen_at
        ) VALUES (?1, 'file', ?2, ?3, 0, ?4, ?5, ?6, ?3)
        "#,
    )
    .bind(&hash_str)
    .bind(metadata.size)
    .bind(now)
    .bind(metadata.is_executable)
    .bind(metadata.is_symlink)
    .bind(&metadata.symlink_target)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to insert file object: {e}"),
    })?;

    ensure_file_verification_row(tx, &hash_str).await?;

    Ok(DeduplicationResult {
        hash: hash.clone(),
        was_duplicate: false,
        ref_count: 0,
        space_saved: 0,
    })
}

/// Helper to ensure a `file_verification` row exists.
async fn ensure_file_verification_row(
    tx: &mut Transaction<'_, Sqlite>,
    hash_str: &str,
) -> Result<(), Error> {
    query(
        r#"
        INSERT OR IGNORE INTO file_verification (file_hash, status, attempts, last_checked_at, last_error)
        VALUES (?1, 'pending', 0, NULL, NULL)
        "#,
    )
    .bind(hash_str)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to ensure verification row: {e}"),
    })?;
    Ok(())
}

/// Insert a package file entry for a `state_packages` row.
///
/// # Errors
///
/// Returns an error if the database operations fail or if the package ID is unknown.
pub async fn add_package_file_entry(
    tx: &mut Transaction<'_, Sqlite>,
    package_id: i64,
    file_ref: &FileReference,
) -> Result<i64, Error> {
    let pv_row = query("SELECT package_version_id AS id FROM state_packages WHERE id = ?1")
        .bind(package_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to resolve package version id: {e}"),
        })?;
    let Some(pv_id) = pv_row.map(|r| r.get::<i64, _>("id")) else {
        return Err(StateError::DatabaseError {
            message: format!("unknown state package id {package_id}"),
        }
        .into());
    };

    let hash_str = file_ref.hash.to_hex();
    query(
        r#"
        INSERT OR IGNORE INTO package_files
          (package_version_id, file_hash, rel_path, mode, uid, gid, mtime)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(pv_id)
    .bind(&hash_str)
    .bind(&file_ref.relative_path)
    .bind(file_ref.metadata.permissions as i64)
    .bind(file_ref.metadata.uid as i64)
    .bind(file_ref.metadata.gid as i64)
    .bind(file_ref.metadata.mtime)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to insert package file entry: {e}"),
    })?;

    let row = query(
        r#"
        SELECT id FROM package_files
        WHERE package_version_id = ?1 AND rel_path = ?2
        "#,
    )
    .bind(pv_id)
    .bind(&file_ref.relative_path)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.get("id"))
}

/// Decrement a file refcount and return the new value.
///
/// # Errors
///
/// Returns an error if the database operations fail.
pub async fn decrement_file_object_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<i64, Error> {
    query("UPDATE cas_objects SET ref_count = ref_count - 1 WHERE hash = ?1 AND kind = 'file'")
        .bind(hash)
        .execute(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to decrement file refcount: {e}"),
        })?;

    let row = query("SELECT ref_count FROM cas_objects WHERE hash = ?1 AND kind = 'file'")
        .bind(hash)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to fetch file refcount: {e}"),
        })?;
    Ok(row.map(|r| r.get("ref_count")).unwrap_or(0))
}

/// Increment a file refcount and return the new value.
///
/// # Errors
///
/// Returns an error if the database operations fail.
pub async fn increment_file_object_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<i64, Error> {
    query(
        "UPDATE cas_objects SET ref_count = ref_count + 1, last_seen_at = strftime('%s','now') WHERE hash = ?1 AND kind = 'file'",
    )
    .bind(hash)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to increment file refcount: {e}"),
    })?;

    let row = query("SELECT ref_count FROM cas_objects WHERE hash = ?1 AND kind = 'file'")
        .bind(hash)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to fetch file refcount: {e}"),
        })?;
    Ok(row.map(|r| r.get("ref_count")).unwrap_or(0))
}

/// Decrement all file refs for the given state package ID.
///
/// # Errors
///
/// Returns an error if the database operations fail.
pub async fn decrement_file_object_refs_for_package(
    tx: &mut Transaction<'_, Sqlite>,
    package_id: i64,
) -> Result<usize, Error> {
    let rows = query(
        r#"
        SELECT pf.file_hash
        FROM state_packages sp
        JOIN package_files pf ON pf.package_version_id = sp.package_version_id
        WHERE sp.id = ?1
        "#,
    )
    .bind(package_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to list package file hashes: {e}"),
    })?;

    let mut count = 0usize;
    for row in rows {
        let hash: String = row.get("file_hash");
        let _ = decrement_file_object_ref(tx, &hash).await?;
        count += 1;
    }
    Ok(count)
}

/// Force set a file object's refcount.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn set_file_object_ref_count(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
    count: i64,
) -> Result<u64, Error> {
    let res = query(
        "UPDATE cas_objects SET ref_count = ?1 WHERE hash = ?2 AND kind = 'file' AND ref_count <> ?1",
    )
    .bind(count)
    .bind(hash)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to set file refcount: {e}"),
    })?;
    Ok(res.rows_affected())
}

/// Fetch a file object.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_file_object(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &Hash,
) -> Result<Option<FileObject>, Error> {
    let hash_str = hash.to_hex();
    let row = query(
        r#"
        SELECT hash, size_bytes AS size, created_at, ref_count,
               is_executable, is_symlink, symlink_target
        FROM cas_objects
        WHERE hash = ?1 AND kind = 'file'
        "#,
    )
    .bind(&hash_str)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to fetch file object: {e}"),
    })?;

    Ok(row.map(|r| FileObject {
        hash: r.get("hash"),
        size: r.get("size"),
        created_at: r.get("created_at"),
        ref_count: r.get("ref_count"),
        is_executable: r.get("is_executable"),
        is_symlink: r.get("is_symlink"),
        symlink_target: r.get("symlink_target"),
    }))
}

/// Fetch all file objects.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_all_file_objects(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<FileObject>, Error> {
    let rows = query(
        r#"
        SELECT hash, size_bytes AS size, created_at, ref_count,
               is_executable, is_symlink, symlink_target
        FROM cas_objects
        WHERE kind = 'file'
        "#,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to list file objects: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| FileObject {
            hash: r.get("hash"),
            size: r.get("size"),
            created_at: r.get("created_at"),
            ref_count: r.get("ref_count"),
            is_executable: r.get("is_executable"),
            is_symlink: r.get("is_symlink"),
            symlink_target: r.get("symlink_target"),
        })
        .collect())
}

/// Hash -> last reference timestamp from states.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_file_last_ref_map(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<HashMap<String, i64>, Error> {
    let rows = query(
        r#"
        SELECT pf.file_hash AS hash, COALESCE(MAX(s.created_at), 0) AS last_ref
        FROM package_files pf
        JOIN state_packages sp ON sp.package_version_id = pf.package_version_id
        JOIN states s ON s.id = sp.state_id
        GROUP BY pf.file_hash
        "#,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to build file last-ref map: {e}"),
    })?;

    let mut map = HashMap::new();
    for r in rows {
        map.insert(r.get("hash"), r.get("last_ref"));
    }
    Ok(map)
}

/// Fetch file entries for a state package ID.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_package_file_entries(
    tx: &mut Transaction<'_, Sqlite>,
    package_id: i64,
) -> Result<Vec<PackageFileEntry>, Error> {
    let rows = query(
        r#"
        SELECT
          pf.id,
          sp.id AS package_id,
          pf.file_hash,
          pf.rel_path AS relative_path,
          pf.mode      AS permissions,
          pf.uid,
          pf.gid,
          pf.mtime
        FROM state_packages sp
        JOIN package_files pf ON pf.package_version_id = sp.package_version_id
        WHERE sp.id = ?1
        ORDER BY pf.rel_path
        "#,
    )
    .bind(package_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to list package file entries: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| PackageFileEntry {
            id: r.get("id"),
            package_id: r.get("package_id"),
            file_hash: r.get("file_hash"),
            relative_path: r.get("relative_path"),
            permissions: r.get("permissions"),
            uid: r.get("uid"),
            gid: r.get("gid"),
            mtime: r.get("mtime"),
        })
        .collect())
}

/// Fetch file entries by hash.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_file_entries_by_hash(
    tx: &mut Transaction<'_, Sqlite>,
    file_hash: &str,
) -> Result<Vec<(String, String, String)>, Error> {
    let rows = query(
        r#"
        SELECT DISTINCT
          pf.rel_path,
          pv.name AS package_name,
          pv.version AS package_version
        FROM package_files pf
        JOIN package_versions pv ON pv.id = pf.package_version_id
        WHERE pf.file_hash = ?1
        ORDER BY pf.rel_path
        "#,
    )
    .bind(file_hash)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get entries by hash: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get("rel_path"),
                r.get("package_name"),
                r.get("package_version"),
            )
        })
        .collect())
}

/// Update (or insert) an mtime tracker entry.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn update_file_mtime(
    tx: &mut Transaction<'_, Sqlite>,
    file_path: &str,
    verified_mtime: i64,
) -> Result<(), Error> {
    query(
        r#"
        INSERT INTO file_mtime_tracker (file_path, last_verified_mtime, created_at, updated_at)
        VALUES (?1, ?2, strftime('%s','now'), strftime('%s','now'))
        ON CONFLICT(file_path) DO UPDATE SET
            last_verified_mtime = excluded.last_verified_mtime,
            updated_at = strftime('%s','now')
        "#,
    )
    .bind(file_path)
    .bind(verified_mtime)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to update mtime tracker: {e}"),
    })?;
    Ok(())
}

/// Fetch package file entries for a state + name + version.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_package_file_entries_by_name(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &uuid::Uuid,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<PackageFileEntry>, Error> {
    let rows = query(
        r#"
        SELECT
          pf.id,
          sp.id AS package_id,
          pf.file_hash,
          pf.rel_path AS relative_path,
          pf.mode      AS permissions,
          pf.uid,
          pf.gid,
          pf.mtime
        FROM state_packages sp
        JOIN package_versions pv ON pv.id = sp.package_version_id
        JOIN package_files pf ON pf.package_version_id = pv.id
        WHERE sp.state_id = ?1 AND pv.name = ?2 AND pv.version = ?3
        ORDER BY pf.rel_path
        "#,
    )
    .bind(state_id.to_string())
    .bind(package_name)
    .bind(package_version)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to fetch package file entries by name: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| PackageFileEntry {
            id: r.get("id"),
            package_id: r.get("package_id"),
            file_hash: r.get("file_hash"),
            relative_path: r.get("relative_path"),
            permissions: r.get("permissions"),
            uid: r.get("uid"),
            gid: r.get("gid"),
            mtime: r.get("mtime"),
        })
        .collect())
}

/// Fetch package file entries across all states for name/version.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_package_file_entries_all_states(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<PackageFileEntry>, Error> {
    let rows = query(
        r#"
        SELECT
          pf.id,
          COALESCE((
            SELECT sp.id
            FROM state_packages sp
            WHERE sp.package_version_id = pv.id
            ORDER BY sp.added_at DESC
            LIMIT 1
          ), 0) AS package_id,
          pf.file_hash,
          pf.rel_path AS relative_path,
          pf.mode      AS permissions,
          pf.uid,
          pf.gid,
          pf.mtime
        FROM package_versions pv
        JOIN package_files pf ON pf.package_version_id = pv.id
        WHERE pv.name = ?1 AND pv.version = ?2
        ORDER BY pf.rel_path
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to fetch package file entries across states: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| PackageFileEntry {
            id: r.get("id"),
            package_id: r.get("package_id"),
            file_hash: r.get("file_hash"),
            relative_path: r.get("relative_path"),
            permissions: r.get("permissions"),
            uid: r.get("uid"),
            gid: r.get("gid"),
            mtime: r.get("mtime"),
        })
        .collect())
}

/// Fetch a file mtime tracker row.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_file_mtime(
    tx: &mut Transaction<'_, Sqlite>,
    file_path: &str,
) -> Result<Option<FileMTimeTracker>, Error> {
    let row = query(
        r#"
        SELECT file_path, last_verified_mtime
        FROM file_mtime_tracker
        WHERE file_path = ?1
        "#,
    )
    .bind(file_path)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to fetch file mtime tracker: {e}"),
    })?;

    Ok(row.map(|r| FileMTimeTracker {
        file_path: r.get("file_path"),
        last_verified_mtime: r.get("last_verified_mtime"),
    }))
}

/// Legacy: mark package file hashed (no-op under schema v2)
///
/// # Errors
///
/// This function does not return an error.
pub async fn mark_package_file_hashed(
    _tx: &mut Transaction<'_, Sqlite>,
    _package_id: i64,
    _computed_hash: &Hash,
) -> Result<(), Error> {
    Ok(())
}

/// Fetch mtime trackers for a package name/version.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_package_file_mtimes(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<FileMTimeTracker>, Error> {
    let rows = query(
        r#"
        SELECT DISTINCT fmt.file_path, fmt.last_verified_mtime
        FROM file_mtime_tracker fmt
        JOIN package_files pf ON pf.rel_path = fmt.file_path
        JOIN package_versions pv ON pv.id = pf.package_version_id
        WHERE pv.name = ?1 AND pv.version = ?2
        ORDER BY fmt.file_path
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to fetch package file mtime trackers: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| FileMTimeTracker {
            file_path: r.get("file_path"),
            last_verified_mtime: r.get("last_verified_mtime"),
        })
        .collect())
}

/// Clear mtime trackers for a package name/version.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn clear_package_mtime_trackers(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<u64, Error> {
    let result = query(
        r#"
        DELETE FROM file_mtime_tracker
        WHERE file_path IN (
            SELECT DISTINCT pf.rel_path
            FROM package_files pf
            JOIN package_versions pv ON pv.id = pf.package_version_id
            WHERE pv.name = ?1 AND pv.version = ?2
        )
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to clear mtime trackers: {e}"),
    })?;
    Ok(result.rows_affected())
}

/// Remove stale mtime trackers older than threshold.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn clear_old_mtime_trackers(
    tx: &mut Transaction<'_, Sqlite>,
    max_age_seconds: i64,
) -> Result<u64, Error> {
    let cutoff = chrono::Utc::now().timestamp() - max_age_seconds;
    let res = query("DELETE FROM file_mtime_tracker WHERE updated_at < ?1")
        .bind(cutoff)
        .execute(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to clear old mtime trackers: {e}"),
        })?;
    Ok(res.rows_affected())
}

/// Fetch file objects that need verification.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_objects_needing_verification(
    tx: &mut Transaction<'_, Sqlite>,
    max_age_seconds: i64,
    max_attempts: i32,
    limit: i64,
) -> Result<Vec<FileObject>, Error> {
    let cutoff = chrono::Utc::now().timestamp() - max_age_seconds;
    let rows = query(
        r#"
        SELECT co.hash,
               co.size_bytes AS size,
               co.created_at,
               co.ref_count,
               co.is_executable,
               co.is_symlink,
               co.symlink_target
        FROM cas_objects co
        LEFT JOIN file_verification fv ON fv.file_hash = co.hash
        WHERE co.kind = 'file' AND co.ref_count > 0
          AND (
                fv.file_hash IS NULL OR
                fv.status = 'pending' OR
                (fv.status = 'verified' AND (fv.last_checked_at IS NULL OR fv.last_checked_at < ?1)) OR
                (fv.status = 'failed' AND fv.attempts < ?2)
          )
        ORDER BY
            CASE COALESCE(fv.status, 'pending')
                WHEN 'failed' THEN 1
                WHEN 'pending' THEN 2
                WHEN 'verified' THEN 3
                ELSE 4
            END,
            COALESCE(fv.last_checked_at, 0) ASC
        LIMIT ?3
        "#,
    )
    .bind(cutoff)
    .bind(max_attempts)
    .bind(limit)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to fetch verification candidates: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| FileObject {
            hash: r.get("hash"),
            size: r.get("size"),
            created_at: r.get("created_at"),
            ref_count: r.get("ref_count"),
            is_executable: r.get("is_executable"),
            is_symlink: r.get("is_symlink"),
            symlink_target: r.get("symlink_target"),
        })
        .collect())
}

/// Update verification status for a hash.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn update_verification_status(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &Hash,
    status: &str,
    error_message: Option<&str>,
) -> Result<(), Error> {
    let hash_str = hash.to_hex();
    let now = chrono::Utc::now().timestamp();
    query(
        r#"
        INSERT INTO file_verification (file_hash, status, attempts, last_checked_at, last_error)
        VALUES (?1, ?2, 1, ?3, ?4)
        ON CONFLICT(file_hash) DO UPDATE SET
            status = excluded.status,
            last_error = excluded.last_error,
            last_checked_at = excluded.last_checked_at,
            attempts = file_verification.attempts + 1
        "#,
    )
    .bind(&hash_str)
    .bind(status)
    .bind(now)
    .bind(error_message)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to update verification status: {e}"),
    })?;
    Ok(())
}

/// Aggregate verification stats for live file objects.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_verification_stats(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<(i64, i64, i64, i64, i64), Error> {
    let row = query(
        r#"
        SELECT
            COUNT(*) AS total_objects,
            SUM(CASE WHEN COALESCE(fv.status, 'pending') = 'verified' THEN 1 ELSE 0 END) AS verified_count,
            SUM(CASE WHEN COALESCE(fv.status, 'pending') = 'pending'  THEN 1 ELSE 0 END) AS pending_count,
            SUM(CASE WHEN COALESCE(fv.status, 'pending') = 'failed'   THEN 1 ELSE 0 END) AS failed_count,
            SUM(CASE WHEN COALESCE(fv.status, 'pending') = 'quarantined' THEN 1 ELSE 0 END) AS quarantined_count
        FROM cas_objects co
        LEFT JOIN file_verification fv ON fv.file_hash = co.hash
        WHERE co.kind = 'file' AND co.ref_count > 0
        "#,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to compute verification stats: {e}"),
    })?;

    Ok((
        row.get("total_objects"),
        row.get("verified_count"),
        row.get("pending_count"),
        row.get("failed_count"),
        row.get("quarantined_count"),
    ))
}

/// Fetch failed verification objects up to limit.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn get_failed_verification_objects(
    tx: &mut Transaction<'_, Sqlite>,
    limit: i64,
) -> Result<Vec<(String, String, i32)>, Error> {
    let rows = query(
        r#"
        SELECT fv.file_hash AS hash,
               COALESCE(fv.last_error, 'unknown error') AS verification_error,
               fv.attempts AS verification_attempts
        FROM file_verification fv
        JOIN cas_objects co ON co.hash = fv.file_hash
        WHERE fv.status = 'failed' AND co.kind = 'file' AND co.ref_count > 0
        ORDER BY fv.attempts DESC, COALESCE(fv.last_checked_at, 0) DESC
        LIMIT ?1
        "#,
    )
    .bind(limit)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to fetch failed verification objects: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get("hash"),
                r.get("verification_error"),
                r.get("verification_attempts"),
            )
        })
        .collect())
}

/// Quarantine a file object.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub async fn quarantine_file_object(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &Hash,
    reason: &str,
) -> Result<(), Error> {
    let hash_str = hash.to_hex();
    let now = chrono::Utc::now().timestamp();
    query(
        r#"
        INSERT INTO file_verification (file_hash, status, attempts, last_checked_at, last_error)
        VALUES (?1, 'quarantined', 1, ?2, ?3)
        ON CONFLICT(file_hash) DO UPDATE SET
            status = 'quarantined',
            last_error = excluded.last_error,
            last_checked_at = excluded.last_checked_at
        "#,
    )
    .bind(&hash_str)
    .bind(now)
    .bind(reason)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to quarantine file object: {e}"),
    })?;
    Ok(())
}

/// Verify a file object and update tracking.
///
/// # Errors
///
/// Returns an error if the database operations or file store operations fail.
pub async fn verify_file_with_tracking(
    tx: &mut Transaction<'_, Sqlite>,
    file_store: &sps2_store::FileStore,
    hash: &Hash,
) -> Result<bool, Error> {
    use sps2_store::FileVerificationResult;

    match file_store.verify_file_detailed(hash).await? {
        FileVerificationResult::Valid => {
            update_verification_status(tx, hash, "verified", None).await?;
            Ok(true)
        }
        FileVerificationResult::Missing => {
            update_verification_status(tx, hash, "failed", Some("file missing from store")).await?;
            Ok(false)
        }
        FileVerificationResult::HashMismatch { expected, actual } => {
            let msg = format!(
                "hash mismatch: expected {}, got {}",
                expected.to_hex(),
                actual.to_hex()
            );
            update_verification_status(tx, hash, "failed", Some(&msg)).await?;
            Ok(false)
        }
        FileVerificationResult::Error { message } => {
            let msg = format!("verification error: {message}");
            update_verification_status(tx, hash, "failed", Some(&msg)).await?;
            Ok(false)
        }
    }
}
