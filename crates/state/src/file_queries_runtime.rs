//! Runtime database queries for file-level content addressable storage
//! This is a temporary implementation until sqlx prepare is run

use crate::file_models::{
    DeduplicationResult, FileMTimeTracker, FileMetadata, FileObject, FileReference,
    PackageFileEntry,
};
use sps2_errors::{Error, StateError};
use sps2_hash::Hash;
use sqlx::{query, Row, Sqlite, Transaction};

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
