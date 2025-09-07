//! Runtime database queries for file-level content addressable storage
//! This is a temporary implementation until sqlx prepare is run

use crate::file_models::{
    DeduplicationResult, FileMTimeTracker, FileMetadata, FileObject, FileReference,
    PackageFileEntry,
};
use sps2_errors::{Error, StateError};
use sps2_hash::Hash;
use sqlx::{query, Row, Sqlite, Transaction};
use std::collections::HashMap;

/// Add a file object to the database
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn add_file_object(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &Hash,
    metadata: &FileMetadata,
) -> Result<DeduplicationResult, Error> {
    let hash_str = hash.to_hex();
    let now = chrono::Utc::now().timestamp();

    // Check if file already exists
    let existing_row = query(
        r#"
        SELECT 
            hash,
            size,
            created_at,
            ref_count,
            is_executable,
            is_symlink,
            symlink_target
        FROM file_objects
        WHERE hash = ?
        "#,
    )
    .bind(&hash_str)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to check existing file object: {e}"),
    })?;

    if let Some(row) = existing_row {
        let ref_count: i64 = row.get("ref_count");

        // Increment reference count
        query("UPDATE file_objects SET ref_count = ref_count + 1 WHERE hash = ?")
            .bind(&hash_str)
            .execute(&mut **tx)
            .await
            .map_err(|e| StateError::DatabaseError {
                message: format!("failed to increment ref count: {e}"),
            })?;

        Ok(DeduplicationResult {
            hash: hash.clone(),
            was_duplicate: true,
            ref_count: ref_count + 1,
            space_saved: metadata.size,
        })
    } else {
        // Insert new file object
        query(
            r#"
            INSERT INTO file_objects (
                hash, size, created_at, ref_count, 
                is_executable, is_symlink, symlink_target
            ) VALUES (?, ?, ?, 1, ?, ?, ?)
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

        Ok(DeduplicationResult {
            hash: hash.clone(),
            was_duplicate: false,
            ref_count: 1,
            space_saved: 0,
        })
    }
}

/// Add a package file entry
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn add_package_file_entry(
    tx: &mut Transaction<'_, Sqlite>,
    package_id: i64,
    file_ref: &FileReference,
) -> Result<i64, Error> {
    let hash_str = file_ref.hash.to_hex();

    query(
        r#"
        INSERT INTO package_file_entries (
            package_id, file_hash, relative_path, permissions,
            uid, gid, mtime
        ) VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(package_id)
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

    // Get the last insert rowid
    let row = query("SELECT last_insert_rowid() as id")
        .fetch_one(&mut **tx)
        .await?;

    Ok(row.get("id"))
}

/// Decrement a file object's refcount by 1, returning the new refcount
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn decrement_file_object_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<i64, Error> {
    query(r#"UPDATE file_objects SET ref_count = ref_count - 1 WHERE hash = ?"#)
        .bind(hash)
        .execute(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to decrement file object refcount: {e}"),
        })?;

    let row = query(r#"SELECT ref_count FROM file_objects WHERE hash = ?"#)
        .bind(hash)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to fetch file object refcount: {e}"),
        })?;

    Ok(row.map(|r| r.get::<i64, _>("ref_count")).unwrap_or(0))
}

/// Increment a file object's refcount by 1, returning the new refcount
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn increment_file_object_ref(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
) -> Result<i64, Error> {
    query(r#"UPDATE file_objects SET ref_count = ref_count + 1 WHERE hash = ?"#)
        .bind(hash)
        .execute(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to increment file object refcount: {e}"),
        })?;

    let row = query(r#"SELECT ref_count FROM file_objects WHERE hash = ?"#)
        .bind(hash)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to fetch file object refcount: {e}"),
        })?;

    Ok(row.map(|r| r.get::<i64, _>("ref_count")).unwrap_or(0))
}

/// Decrement `file_object` refcounts for all entries of a package
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn decrement_file_object_refs_for_package(
    tx: &mut Transaction<'_, Sqlite>,
    package_id: i64,
) -> Result<usize, Error> {
    // Collect all file hashes for this package
    let rows = query(
        r#"
        SELECT file_hash FROM package_file_entries
        WHERE package_id = ?
        "#,
    )
    .bind(package_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to list package file entries for decrement: {e}"),
    })?;

    let mut count = 0usize;
    for r in rows {
        let hash: String = r.get("file_hash");
        let _ = decrement_file_object_ref(tx, &hash).await?;
        count += 1;
    }
    Ok(count)
}

/// Set a file object's refcount to an exact value
///
/// # Errors
///
/// Returns the number of rows updated
pub async fn set_file_object_ref_count(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &str,
    count: i64,
) -> Result<u64, Error> {
    let res = query(r#"UPDATE file_objects SET ref_count = ? WHERE hash = ? AND ref_count <> ?"#)
        .bind(count)
        .bind(hash)
        .bind(count)
        .execute(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to set file object refcount: {e}"),
        })?;
    Ok(res.rows_affected())
}

/// Get file object by hash
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_file_object(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &Hash,
) -> Result<Option<FileObject>, Error> {
    let hash_str = hash.to_hex();

    let row = query(
        r#"
        SELECT 
            hash,
            size,
            created_at,
            ref_count,
            is_executable,
            is_symlink,
            symlink_target
        FROM file_objects
        WHERE hash = ?
        "#,
    )
    .bind(&hash_str)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get file object: {e}"),
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

/// Get all file objects
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_all_file_objects(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<FileObject>, Error> {
    let rows = query(
        r#"
        SELECT 
            hash,
            size,
            created_at,
            ref_count,
            is_executable,
            is_symlink,
            symlink_target
        FROM file_objects
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

/// Build a map of file hash -> last reference timestamp across all states
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_file_last_ref_map(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<HashMap<String, i64>, Error> {
    let rows = query(
        r#"
        SELECT pfe.file_hash AS hash, COALESCE(MAX(s.created_at), 0) AS last_ref
        FROM package_file_entries pfe
        JOIN packages p ON p.id = pfe.package_id
        JOIN states s ON s.id = p.state_id
        GROUP BY pfe.file_hash
        "#,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to build file last-ref map: {e}"),
    })?;

    let mut map = HashMap::new();
    for r in rows {
        let hash: String = r.get("hash");
        let last_ref: i64 = r.get("last_ref");
        map.insert(hash, last_ref);
    }
    Ok(map)
}

/// Get all file entries for a package
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_package_file_entries(
    tx: &mut Transaction<'_, Sqlite>,
    package_id: i64,
) -> Result<Vec<PackageFileEntry>, Error> {
    let rows = query(
        r#"
        SELECT 
            id,
            package_id,
            file_hash,
            relative_path,
            permissions,
            uid,
            gid,
            mtime
        FROM package_file_entries
        WHERE package_id = ?
        ORDER BY relative_path
        "#,
    )
    .bind(package_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get package files: {e}"),
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

/// Get file entries by hash
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_file_entries_by_hash(
    tx: &mut Transaction<'_, Sqlite>,
    file_hash: &str,
) -> Result<Vec<(String, String, String)>, Error> {
    let rows = query(
        r#"
        SELECT DISTINCT 
            pfe.relative_path,
            p.name as package_name,
            p.version as package_version
        FROM package_file_entries pfe
        JOIN packages p ON p.id = pfe.package_id
        WHERE pfe.file_hash = ?
        ORDER BY pfe.relative_path
        "#,
    )
    .bind(file_hash)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get file entries by hash: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get("relative_path"),
                r.get("package_name"),
                r.get("package_version"),
            )
        })
        .collect())
}

/// Update file modification time tracker
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn update_file_mtime(
    tx: &mut Transaction<'_, Sqlite>,
    file_path: &str,
    verified_mtime: i64,
) -> Result<(), Error> {
    query(
        r#"
        INSERT OR REPLACE INTO file_mtime_tracker (
            file_path, last_verified_mtime
        ) VALUES (?, ?)
        "#,
    )
    .bind(file_path)
    .bind(verified_mtime)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to update file mtime tracker: {e}"),
    })?;

    Ok(())
}

/// Get package file entries by package name and version
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_package_file_entries_by_name(
    tx: &mut Transaction<'_, Sqlite>,
    state_id: &uuid::Uuid,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<PackageFileEntry>, Error> {
    let state_id_str = state_id.to_string();

    // First get the package ID
    let package_id: Option<i64> = query(
        r#"
        SELECT id FROM packages
        WHERE state_id = ? AND name = ? AND version = ?
        "#,
    )
    .bind(&state_id_str)
    .bind(package_name)
    .bind(package_version)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get package id: {e}"),
    })?
    .map(|r| r.get("id"));

    match package_id {
        Some(id) => get_package_file_entries(tx, id).await,
        None => Ok(Vec::new()),
    }
}

/// Get package file entries by package name and version across all states
///
/// This searches for file entries across all states, not just a specific one.
/// Useful when a package's file entries haven't been propagated to the current state.
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_package_file_entries_all_states(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<PackageFileEntry>, Error> {
    let rows = query(
        r#"
        SELECT DISTINCT 
            pfe.id,
            pfe.package_id,
            pfe.file_hash,
            pfe.relative_path,
            pfe.permissions,
            pfe.uid,
            pfe.gid,
            pfe.mtime
        FROM package_file_entries pfe
        JOIN packages p ON pfe.package_id = p.id
        WHERE p.name = ? AND p.version = ?
        ORDER BY pfe.relative_path
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get package file entries across all states: {e}"),
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

/// Get file modification time tracker entry
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_file_mtime(
    tx: &mut Transaction<'_, Sqlite>,
    file_path: &str,
) -> Result<Option<FileMTimeTracker>, Error> {
    let row = query(
        r#"
        SELECT 
            file_path,
            last_verified_mtime
        FROM file_mtime_tracker
        WHERE file_path = ?
        "#,
    )
    .bind(file_path)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get file mtime tracker: {e}"),
    })?;

    Ok(row.map(|r| FileMTimeTracker {
        file_path: r.get("file_path"),
        last_verified_mtime: r.get("last_verified_mtime"),
    }))
}

/// Mark package as having file-level hashes
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn mark_package_file_hashed(
    tx: &mut Transaction<'_, Sqlite>,
    package_id: i64,
    computed_hash: &Hash,
) -> Result<(), Error> {
    let hash_str = computed_hash.to_hex();

    query(
        r#"
        UPDATE packages 
        SET has_file_hashes = 1, computed_hash = ?
        WHERE id = ?
        "#,
    )
    .bind(&hash_str)
    .bind(package_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to mark package as file-hashed: {e}"),
    })?;

    Ok(())
}

/// Get all mtime trackers for files in a package
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_package_file_mtimes(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<FileMTimeTracker>, Error> {
    let rows = query(
        r#"
        SELECT DISTINCT
            fmt.file_path,
            fmt.last_verified_mtime
        FROM file_mtime_tracker fmt
        JOIN package_file_entries pfe ON pfe.relative_path = fmt.file_path
        JOIN packages p ON p.id = pfe.package_id
        WHERE p.name = ? AND p.version = ?
        ORDER BY fmt.file_path
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get package file mtime trackers: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| FileMTimeTracker {
            file_path: r.get("file_path"),
            last_verified_mtime: r.get("last_verified_mtime"),
        })
        .collect())
}

/// Clear mtime trackers for a package
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn clear_package_mtime_trackers(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<u64, Error> {
    // Clear mtime trackers for all files associated with this package
    let result = query(
        r#"
        DELETE FROM file_mtime_tracker
        WHERE file_path IN (
            SELECT DISTINCT pfe.relative_path
            FROM package_file_entries pfe
            JOIN packages p ON p.id = pfe.package_id
            WHERE p.name = ? AND p.version = ?
        )
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to clear package mtime trackers: {e}"),
    })?;

    Ok(result.rows_affected())
}

/// Clear old mtime tracker entries
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn clear_old_mtime_trackers(
    tx: &mut Transaction<'_, Sqlite>,
    max_age_seconds: i64,
) -> Result<u64, Error> {
    let cutoff_time = chrono::Utc::now().timestamp() - max_age_seconds;

    let result = query(
        r#"
        DELETE FROM file_mtime_tracker
        WHERE updated_at < ?
        "#,
    )
    .bind(cutoff_time)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to clear old mtime trackers: {e}"),
    })?;

    Ok(result.rows_affected())
}

/// Get file objects that need verification
///
/// Returns objects that are either:
/// - Never verified (`verification_status` = 'pending')
/// - Verified but older than the threshold
/// - Failed verification with attempts below `max_attempts`
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_objects_needing_verification(
    tx: &mut Transaction<'_, Sqlite>,
    max_age_seconds: i64,
    max_attempts: i32,
    limit: i64,
) -> Result<Vec<FileObject>, Error> {
    let cutoff_time = chrono::Utc::now().timestamp() - max_age_seconds;

    let rows = query(
        r#"
        SELECT 
            hash,
            size,
            created_at,
            ref_count,
            is_executable,
            is_symlink,
            symlink_target,
            last_verified_at,
            verification_status,
            verification_error,
            verification_attempts
        FROM file_objects
        WHERE ref_count > 0 AND (
            verification_status = 'pending' OR
            (verification_status = 'verified' AND (last_verified_at IS NULL OR last_verified_at < ?)) OR
            (verification_status = 'failed' AND verification_attempts < ?)
        )
        ORDER BY 
            CASE verification_status
                WHEN 'failed' THEN 1
                WHEN 'pending' THEN 2
                WHEN 'verified' THEN 3
                ELSE 4
            END,
            last_verified_at ASC NULLS FIRST
        LIMIT ?
        "#,
    )
    .bind(cutoff_time)
    .bind(max_attempts)
    .bind(limit)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get objects needing verification: {e}"),
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

/// Update verification status for a file object
///
/// # Errors
///
/// Returns an error if the database operation fails
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
        UPDATE file_objects 
        SET 
            verification_status = ?,
            verification_error = ?,
            last_verified_at = ?,
            verification_attempts = verification_attempts + 1
        WHERE hash = ?
        "#,
    )
    .bind(status)
    .bind(error_message)
    .bind(now)
    .bind(&hash_str)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to update verification status: {e}"),
    })?;

    Ok(())
}

/// Get verification statistics for the store
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_verification_stats(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<(i64, i64, i64, i64, i64), Error> {
    let row = query(
        r#"
        SELECT 
            COUNT(*) as total_objects,
            SUM(CASE WHEN verification_status = 'verified' THEN 1 ELSE 0 END) as verified_count,
            SUM(CASE WHEN verification_status = 'pending' THEN 1 ELSE 0 END) as pending_count,
            SUM(CASE WHEN verification_status = 'failed' THEN 1 ELSE 0 END) as failed_count,
            SUM(CASE WHEN verification_status = 'quarantined' THEN 1 ELSE 0 END) as quarantined_count
        FROM file_objects
        WHERE ref_count > 0
        "#,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get verification stats: {e}"),
    })?;

    Ok((
        row.get("total_objects"),
        row.get("verified_count"),
        row.get("pending_count"),
        row.get("failed_count"),
        row.get("quarantined_count"),
    ))
}

/// Get objects with failed verification
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_failed_verification_objects(
    tx: &mut Transaction<'_, Sqlite>,
    limit: i64,
) -> Result<Vec<(String, String, i32)>, Error> {
    let rows = query(
        r#"
        SELECT 
            hash,
            verification_error,
            verification_attempts
        FROM file_objects
        WHERE verification_status = 'failed' AND ref_count > 0
        ORDER BY verification_attempts DESC, last_verified_at DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get failed verification objects: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get("hash"),
                r.get::<Option<String>, _>("verification_error")
                    .unwrap_or_else(|| "unknown error".to_string()),
                r.get("verification_attempts"),
            )
        })
        .collect())
}

/// Quarantine a file object (mark as quarantined)
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn quarantine_file_object(
    tx: &mut Transaction<'_, Sqlite>,
    hash: &Hash,
    reason: &str,
) -> Result<(), Error> {
    let hash_str = hash.to_hex();
    let now = chrono::Utc::now().timestamp();

    query(
        r#"
        UPDATE file_objects 
        SET 
            verification_status = 'quarantined',
            verification_error = ?,
            last_verified_at = ?
        WHERE hash = ?
        "#,
    )
    .bind(reason)
    .bind(now)
    .bind(&hash_str)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to quarantine file object: {e}"),
    })?;

    Ok(())
}

/// Verify a file object and update database tracking
///
/// This function performs verification using the store and updates the database with the result.
/// It's designed to be used by the store verification system.
///
/// # Errors
/// Returns an error if verification or database operations fail
pub async fn verify_file_with_tracking(
    tx: &mut Transaction<'_, Sqlite>,
    file_store: &sps2_store::FileStore,
    hash: &Hash,
) -> Result<bool, Error> {
    use sps2_store::FileVerificationResult;

    // Perform the actual verification
    let verification_result = file_store.verify_file_detailed(hash).await?;

    match verification_result {
        FileVerificationResult::Valid => {
            // File is valid - mark as verified
            update_verification_status(tx, hash, "verified", None).await?;
            Ok(true)
        }
        FileVerificationResult::Missing => {
            // File is missing - mark as failed
            let error_msg = "file missing from store";
            update_verification_status(tx, hash, "failed", Some(error_msg)).await?;
            Ok(false)
        }
        FileVerificationResult::HashMismatch { expected, actual } => {
            // File hash mismatch - mark as failed
            let error_msg = format!(
                "hash mismatch: expected {}, got {}",
                expected.to_hex(),
                actual.to_hex()
            );
            update_verification_status(tx, hash, "failed", Some(&error_msg)).await?;
            Ok(false)
        }
        FileVerificationResult::Error { message } => {
            // Verification failed due to error - mark as failed
            let error_msg = format!("verification error: {message}");
            update_verification_status(tx, hash, "failed", Some(&error_msg)).await?;
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::StateManager;
    use tempfile::TempDir;

    async fn mk_state() -> (TempDir, StateManager) {
        let td = TempDir::new().expect("tempdir");
        let mgr = StateManager::new(td.path()).await.expect("state new");
        (td, mgr)
    }

    #[tokio::test]
    async fn decrement_file_object_ref_allows_below_zero_and_preserves_row() {
        let (_td, state) = mk_state().await;
        let mut tx = state.begin_transaction().await.expect("tx");

        // Seed a file object with ref_count = 1
        let h = Hash::from_data(b"file-A");
        let meta = FileMetadata::regular_file(10, 0o644);
        let _ = add_file_object(&mut tx, &h, &meta).await.expect("add");

        // Decrement to zero
        let c0 = decrement_file_object_ref(&mut tx, &h.to_hex())
            .await
            .expect("dec0");
        assert_eq!(c0, 0);

        // Decrement again (may go negative, row persists)
        let c1 = decrement_file_object_ref(&mut tx, &h.to_hex())
            .await
            .expect("dec1");
        assert_eq!(c1, -1);

        // Row still exists
        let fo = get_file_object(&mut tx, &h).await.expect("get");
        assert!(fo.is_some());
    }

    #[tokio::test]
    async fn decrement_file_object_refs_for_package_counts_entries() {
        let (_td, state) = mk_state().await;
        let mut tx = state.begin_transaction().await.expect("tx");
        let sid = state.get_current_state_id().await.expect("state id");

        // Add package and two files
        let pkg_id = crate::queries::add_package(&mut tx, &sid, "pkg", "1.0.0", "deadbeef", 1)
            .await
            .expect("add pkg");

        let h1 = Hash::from_data(b"F1");
        let h2 = Hash::from_data(b"F2");
        let meta1 = FileMetadata::regular_file(5, 0o644);
        let meta2 = FileMetadata::regular_file(6, 0o644);
        let _ = add_file_object(&mut tx, &h1, &meta1).await.expect("add f1");
        let _ = add_file_object(&mut tx, &h2, &meta2).await.expect("add f2");

        let fr1 = FileReference {
            package_id: pkg_id,
            relative_path: "bin/a".to_string(),
            hash: h1.clone(),
            metadata: meta1.clone(),
        };
        let fr2 = FileReference {
            package_id: pkg_id,
            relative_path: "bin/b".to_string(),
            hash: h2.clone(),
            metadata: meta2.clone(),
        };
        let _ = add_package_file_entry(&mut tx, pkg_id, &fr1)
            .await
            .expect("pfe1");
        let _ = add_package_file_entry(&mut tx, pkg_id, &fr2)
            .await
            .expect("pfe2");

        let dec = decrement_file_object_refs_for_package(&mut tx, pkg_id)
            .await
            .expect("dec refs");
        assert_eq!(dec, 2);

        let first_obj = get_file_object(&mut tx, &h1)
            .await
            .expect("get f1")
            .unwrap();
        let second_obj = get_file_object(&mut tx, &h2)
            .await
            .expect("get f2")
            .unwrap();
        assert_eq!(first_obj.ref_count, 0);
        assert_eq!(second_obj.ref_count, 0);
    }

    #[tokio::test]
    async fn set_file_object_refcount_is_idempotent() {
        let (_td, state) = mk_state().await;
        let mut tx = state.begin_transaction().await.expect("tx");
        let h = Hash::from_data(b"file-Z");
        let meta = FileMetadata::regular_file(3, 0o644);
        let _ = add_file_object(&mut tx, &h, &meta).await.expect("add");
        let _ = increment_file_object_ref(&mut tx, &h.to_hex())
            .await
            .expect("inc");
        let _ = increment_file_object_ref(&mut tx, &h.to_hex())
            .await
            .expect("inc2");

        let updated = set_file_object_ref_count(&mut tx, &h.to_hex(), 2)
            .await
            .expect("set1");
        assert!(updated > 0);
        let updated2 = set_file_object_ref_count(&mut tx, &h.to_hex(), 2)
            .await
            .expect("set2");
        assert_eq!(updated2, 0);
    }
}
