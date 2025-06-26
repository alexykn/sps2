//! File content verification logic

use crate::types::Discrepancy;
use sps2_errors::{Error, OpsError};
use sps2_hash::Hash;
use sps2_store::{PackageStore, StoredPackage};
use std::path::Path;

/// Verify file content hash
pub async fn verify_file_content(
    store: &PackageStore,
    file_path: &Path,
    package: &sps2_state::models::Package,
    relative_path: &str,
    discrepancies: &mut Vec<Discrepancy>,
) -> Result<(), Error> {
    // Skip hash verification for directories and symlinks
    let metadata = tokio::fs::symlink_metadata(file_path).await?;
    if metadata.is_dir() || metadata.is_symlink() {
        return Ok(());
    }

    // Load package from store to get expected file hash
    let package_hash = Hash::from_hex(&package.hash).map_err(|e| OpsError::OperationFailed {
        message: format!("Invalid package hash: {e}"),
    })?;
    let store_path = store.package_path(&package_hash);

    if !store_path.exists() {
        // Can't verify without store package
        return Ok(());
    }

    let stored_package = StoredPackage::load(&store_path).await?;
    let expected_file_path = stored_package.files_path().join(relative_path);

    if !expected_file_path.exists() {
        // File not in store, can't verify
        return Ok(());
    }

    // Calculate actual file hash
    let actual_hash = Hash::hash_file(file_path).await?;
    let expected_hash = Hash::hash_file(&expected_file_path).await?;

    if actual_hash != expected_hash {
        discrepancies.push(Discrepancy::CorruptedFile {
            package_name: package.name.clone(),
            package_version: package.version.clone(),
            file_path: relative_path.to_string(),
            expected_hash: expected_hash.to_hex(),
            actual_hash: actual_hash.to_hex(),
        });
    }

    Ok(())
}
