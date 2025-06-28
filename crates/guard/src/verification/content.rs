//! File content verification logic

use crate::types::Discrepancy;
use sps2_errors::{Error, OpsError};
use sps2_events::{Event, EventSender};
use sps2_hash::Hash;
use std::path::Path;

/// Parameters for content verification to avoid too many function arguments
pub struct ContentVerificationParams<'a> {
    /// State manager for database operations
    pub state_manager: &'a sps2_state::StateManager,
    /// Current state ID being verified
    pub state_id: &'a uuid::Uuid,
    /// Full path to the file being verified
    pub file_path: &'a Path,
    /// Package information
    pub package: &'a sps2_state::models::Package,
    /// Relative path within the package
    pub relative_path: &'a str,
    /// Collection to add discrepancies to
    pub discrepancies: &'a mut Vec<Discrepancy>,
    /// Operation ID for event tracking
    pub operation_id: &'a str,
    /// Optional event sender for progress reporting
    pub tx: Option<&'a EventSender>,
}

/// Helper function to add a discrepancy and emit the corresponding event
fn add_discrepancy_with_event(
    discrepancies: &mut Vec<Discrepancy>,
    discrepancy: Discrepancy,
    operation_id: &str,
    tx: Option<&EventSender>,
) {
    // Determine severity and user message based on discrepancy type
    let (severity, user_message, auto_heal_available) = match &discrepancy {
        Discrepancy::CorruptedFile { .. } => {
            ("high", "File content does not match expected hash", true)
        }
        _ => ("medium", "Unknown discrepancy", false),
    };

    // Extract file path and package info
    let (file_path, package, package_version) = match &discrepancy {
        Discrepancy::CorruptedFile {
            file_path,
            package_name,
            package_version,
            ..
        } => (
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

/// Verify file content hash using database
pub async fn verify_file_content(params: ContentVerificationParams<'_>) -> Result<(), Error> {
    let ContentVerificationParams {
        state_manager,
        state_id,
        file_path,
        package,
        relative_path,
        discrepancies,
        operation_id,
        tx,
    } = params;
    // Skip hash verification for directories and symlinks
    let metadata = tokio::fs::symlink_metadata(file_path).await?;
    if metadata.is_dir() || metadata.is_symlink() {
        return Ok(());
    }

    // Get expected hash from database
    let mut db_tx = state_manager.begin_transaction().await?;

    // Get package file entries with hashes
    let file_entries = sps2_state::queries::get_package_file_entries_by_name(
        &mut db_tx,
        state_id,
        &package.name,
        &package.version,
    )
    .await?;

    db_tx.commit().await?;

    // Find the file entry for this relative path
    // Need to handle the opt/pm/live prefix stripping
    let stripped_path = if relative_path.starts_with("opt/pm/live/") {
        relative_path.strip_prefix("opt/pm/live/").unwrap()
    } else {
        relative_path
    };

    let file_entry = file_entries.iter().find(|entry| {
        let entry_path = if entry.relative_path.starts_with("opt/pm/live/") {
            entry.relative_path.strip_prefix("opt/pm/live/").unwrap()
        } else {
            &entry.relative_path
        };
        entry_path == stripped_path
    });

    let Some(entry) = file_entry else {
        // No hash in database for this file - this might be a legacy package
        // or the file-level data hasn't been populated yet
        return Ok(());
    };

    // Calculate actual file hash
    let actual_hash = Hash::hash_file(file_path).await?;
    let expected_hash =
        Hash::from_hex(&entry.file_hash).map_err(|e| OpsError::OperationFailed {
            message: format!("Invalid file hash in database: {e}"),
        })?;

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
