//! Runtime database queries for file-level content addressable storage
//! This is a temporary implementation until sqlx prepare is run

use crate::file_models::{
    DeduplicationResult, FileMetadata, FileObject, FileReference, FileVerificationCache,
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

/// Update verification cache
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn update_verification_cache(
    tx: &mut Transaction<'_, Sqlite>,
    file_hash: &Hash,
    installed_path: &str,
    is_valid: bool,
    error_message: Option<&str>,
) -> Result<(), Error> {
    let hash_str = file_hash.to_hex();
    let now = chrono::Utc::now().timestamp();

    query(
        r#"
        INSERT OR REPLACE INTO file_verification_cache (
            file_hash, installed_path, verified_at, is_valid, error_message
        ) VALUES (?, ?, ?, ?, ?)
        "#,
    )
    .bind(&hash_str)
    .bind(installed_path)
    .bind(now)
    .bind(is_valid)
    .bind(error_message)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to update verification cache: {e}"),
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

/// Get verification cache entry
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_verification_cache(
    tx: &mut Transaction<'_, Sqlite>,
    file_hash: &Hash,
    installed_path: &str,
) -> Result<Option<FileVerificationCache>, Error> {
    let hash_str = file_hash.to_hex();

    let row = query(
        r#"
        SELECT 
            file_hash,
            installed_path,
            verified_at,
            is_valid,
            error_message
        FROM file_verification_cache
        WHERE file_hash = ? AND installed_path = ?
        "#,
    )
    .bind(&hash_str)
    .bind(installed_path)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get verification cache: {e}"),
    })?;

    Ok(row.map(|r| FileVerificationCache {
        file_hash: r.get("file_hash"),
        installed_path: r.get("installed_path"),
        verified_at: r.get("verified_at"),
        is_valid: r.get("is_valid"),
        error_message: r.get("error_message"),
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

/// Get all cached verification results for files in a package
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn get_package_file_verification_cache(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<Vec<FileVerificationCache>, Error> {
    let rows = query(
        r#"
        SELECT DISTINCT
            fvc.file_hash,
            fvc.installed_path,
            fvc.verified_at,
            fvc.is_valid,
            fvc.error_message
        FROM file_verification_cache fvc
        JOIN package_file_entries pfe ON pfe.file_hash = fvc.file_hash
        JOIN packages p ON p.id = pfe.package_id
        WHERE p.name = ? AND p.version = ?
        ORDER BY fvc.installed_path
        "#,
    )
    .bind(package_name)
    .bind(package_version)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get package verification cache: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| FileVerificationCache {
            file_hash: r.get("file_hash"),
            installed_path: r.get("installed_path"),
            verified_at: r.get("verified_at"),
            is_valid: r.get("is_valid"),
            error_message: r.get("error_message"),
        })
        .collect())
}

/// Clear verification cache for a package
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn clear_package_verification_cache(
    tx: &mut Transaction<'_, Sqlite>,
    package_name: &str,
    package_version: &str,
) -> Result<u64, Error> {
    let result = query(
        r#"
        DELETE FROM file_verification_cache
        WHERE file_hash IN (
            SELECT pfe.file_hash
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
        message: format!("failed to clear package verification cache: {e}"),
    })?;

    Ok(result.rows_affected())
}

/// Clear old verification cache entries
///
/// # Errors
///
/// Returns an error if the database operation fails
pub async fn clear_old_verification_cache(
    tx: &mut Transaction<'_, Sqlite>,
    max_age_seconds: i64,
) -> Result<u64, Error> {
    let cutoff_time = chrono::Utc::now().timestamp() - max_age_seconds;
    
    let result = query(
        r#"
        DELETE FROM file_verification_cache
        WHERE verified_at < ?
        "#,
    )
    .bind(cutoff_time)
    .execute(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to clear old verification cache: {e}"),
    })?;

    Ok(result.rows_affected())
}
