//! File content verification logic

use crate::types::Discrepancy;
use sps2_errors::{Error, OpsError};
use sps2_events::{Event, EventSender};
use sps2_hash::Hash;
use sps2_store::{PackageStore, StoredPackage};
use std::collections::HashMap;
use std::path::Path;

/// Helper function to add a discrepancy and emit the corresponding event
fn add_discrepancy_with_event(
    discrepancies: &mut Vec<Discrepancy>,
    discrepancy: Discrepancy,
    operation_id: &str,
    tx: Option<&EventSender>,
) {
    // Determine severity and user message based on discrepancy type
    let (severity, user_message, auto_heal_available) = match &discrepancy {
        Discrepancy::CorruptedFile { .. } => (
            "high", 
            "File content does not match expected hash",
            true,
        ),
        _ => ("medium", "Unknown discrepancy", false),
    };

    // Extract file path and package info
    let (file_path, package, package_version) = match &discrepancy {
        Discrepancy::CorruptedFile { file_path, package_name, package_version, .. } => (
            file_path.clone(),
            Some(package_name.clone()),
            Some(package_version.to_string()),
        ),
        _ => ("unknown".to_string(), None, None),
    };

    // Emit the event if we have a sender
    if let Some(sender) = tx {
        let _ = sender.send(Event::GuardDiscrepancyFound {
            operation_id: operation_id.to_string(),
            discrepancy_type: format!("{:?}", std::mem::discriminant(&discrepancy)),
            severity: severity.to_string(),
            file_path: file_path.clone(),
            package,
            package_version,
            user_message: user_message.to_string(),
            technical_details: format!("{:?}", discrepancy),
            auto_heal_available,
            requires_confirmation: false,
            estimated_fix_time_seconds: Some(30),
        });
    }

    // Add to discrepancies list
    discrepancies.push(discrepancy);
}

/// Verify file content hash
pub async fn verify_file_content(
    store: &PackageStore,
    file_path: &Path,
    package: &sps2_state::models::Package,
    relative_path: &str,
    discrepancies: &mut Vec<Discrepancy>,
    operation_id: &str,
    tx: Option<&EventSender>,
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
        add_discrepancy_with_event(
            discrepancies,
            Discrepancy::CorruptedFile {
                package_name: package.name.clone(),
                package_version: package.version.clone(),
                file_path: relative_path.to_string(),
                expected_hash: expected_hash.to_hex(),
                actual_hash: actual_hash.to_hex(),
            },
            operation_id,
            tx,
        );
    }

    Ok(())
}
