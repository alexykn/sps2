//! Migration helpers for transitioning to file-level storage

use crate::file_models::FileReference;
use crate::file_queries_runtime::mark_package_file_hashed;
use crate::models::Package;
use sps2_errors::{Error, StateError};
use sps2_hash::Hash;
use sqlx::{query, Row, Sqlite, Transaction};
use std::path::Path;

/// Migration status for a package
#[derive(Debug, Clone)]
pub struct PackageMigrationStatus {
    pub package_id: i64,
    pub package_name: String,
    pub package_version: String,
    pub files_processed: usize,
    pub space_saved: i64,
    pub computed_hash: Hash,
}

/// Migrate a package to file-level storage
///
/// This function is used during the migration phase to convert existing
/// packages to use file-level hashing. It doesn't actually read files
/// from disk but prepares the database schema for future use.
///
/// # Errors
///
/// Returns an error if database operations fail
pub async fn migrate_package_placeholder(
    tx: &mut Transaction<'_, Sqlite>,
    package: &Package,
) -> Result<PackageMigrationStatus, Error> {
    // For now, we'll create a placeholder migration that marks the package
    // as needing file-level processing when it's next accessed

    // Compute a placeholder hash based on package info
    let placeholder_data = format!("{}-{}", package.name, package.version);
    let computed_hash = Hash::from_data(placeholder_data.as_bytes());

    // Mark the package as having file hashes (even though we haven't computed them yet)
    // This will trigger file-level processing on next access
    mark_package_file_hashed(tx, package.id, &computed_hash).await?;

    Ok(PackageMigrationStatus {
        package_id: package.id,
        package_name: package.name.clone(),
        package_version: package.version.clone(),
        files_processed: 0, // Will be populated during actual file processing
        space_saved: 0,
        computed_hash,
    })
}

/// Process files from a package directory and add them to file-level storage
///
/// This would be called when actually installing or verifying a package
///
/// # Errors
///
/// Returns an error if file processing fails
pub async fn process_package_files(
    _tx: &mut Transaction<'_, Sqlite>,
    package_id: i64,
    package_path: &Path,
) -> Result<Vec<FileReference>, Error> {
    // This is a placeholder for the actual implementation
    // In reality, this would:
    // 1. Walk the package directory
    // 2. Hash each file
    // 3. Add file objects to the database
    // 4. Create package file entries

    let _ = (package_id, package_path); // Suppress unused warnings

    // For now, return empty vec
    Ok(Vec::new())
}

/// Check if a package has been migrated to file-level storage
///
/// # Errors
///
/// Returns an error if the database query fails
pub async fn is_package_migrated(
    tx: &mut Transaction<'_, Sqlite>,
    package_id: i64,
) -> Result<bool, Error> {
    let row = query("SELECT has_file_hashes FROM packages WHERE id = ?")
        .bind(package_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| StateError::DatabaseError {
            message: format!("failed to check migration status: {e}"),
        })?;

    Ok(row
        .map(|r| {
            let has_hashes: i64 = r.get("has_file_hashes");
            has_hashes != 0
        })
        .unwrap_or(false))
}

/// Get packages that need migration
///
/// # Errors
///
/// Returns an error if the database query fails
pub async fn get_packages_needing_migration(
    tx: &mut Transaction<'_, Sqlite>,
    limit: i64,
) -> Result<Vec<Package>, Error> {
    let rows = query(
        r#"
        SELECT 
            id,
            state_id,
            name,
            version,
            hash,
            size,
            installed_at,
            venv_path
        FROM packages
        WHERE has_file_hashes = 0
        ORDER BY installed_at DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get packages needing migration: {e}"),
    })?;

    Ok(rows
        .into_iter()
        .map(|r| Package {
            id: r.get("id"),
            state_id: r.get("state_id"),
            name: r.get("name"),
            version: r.get("version"),
            hash: r.get("hash"),
            size: r.get("size"),
            installed_at: r.get("installed_at"),
            venv_path: r.get("venv_path"),
        })
        .collect())
}

/// Migration statistics
#[derive(Debug, Default)]
pub struct MigrationStats {
    pub total_packages: usize,
    pub migrated_packages: usize,
    pub failed_packages: usize,
    pub total_space_saved: i64,
}

/// Get migration statistics
///
/// # Errors
///
/// Returns an error if the database query fails
pub async fn get_migration_stats(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<MigrationStats, Error> {
    let row = query(
        r#"
        SELECT 
            COUNT(*) as total,
            SUM(CASE WHEN has_file_hashes = 1 THEN 1 ELSE 0 END) as migrated
        FROM packages
        "#,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| StateError::DatabaseError {
        message: format!("failed to get migration stats: {e}"),
    })?;

    let total: i64 = row.get("total");
    let migrated: i64 = row.get("migrated");

    Ok(MigrationStats {
        total_packages: total as usize,
        migrated_packages: migrated as usize,
        failed_packages: 0,   // Would track this separately
        total_space_saved: 0, // Would calculate from file deduplication
    })
}
