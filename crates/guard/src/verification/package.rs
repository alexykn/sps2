//! Package verification logic

use crate::types::{Discrepancy, FileCacheEntry, SymlinkPolicy, VerificationContext};
use crate::verification::content::verify_file_content;
use sps2_errors::Error;
use sps2_events::Event;
use sps2_hash::Hash;
use sps2_state::queries;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

/// Helper function to add a discrepancy and emit the corresponding event
fn add_discrepancy_with_event(
    discrepancies: &mut Vec<Discrepancy>,
    discrepancy: Discrepancy,
    operation_id: &str,
    tx: Option<&sps2_events::EventSender>,
) {
    // Determine severity and user message based on discrepancy type
    let (severity, user_message, auto_heal_available) = match &discrepancy {
        Discrepancy::MissingFile { .. } => (
            "high",
            "File is missing from the filesystem",
            true,
        ),
        Discrepancy::CorruptedFile { .. } => (
            "high", 
            "File content does not match expected hash",
            true,
        ),
        Discrepancy::MissingVenv { .. } => (
            "medium",
            "Python virtual environment is missing",
            true,
        ),
        Discrepancy::OrphanedFile { .. } => (
            "low",
            "File exists but is not tracked by any package",
            false,
        ),
        Discrepancy::TypeMismatch { .. } => (
            "medium",
            "File type does not match expected type",
            true,
        ),
    };

    // Extract file path and package info
    let (file_path, package, package_version) = match &discrepancy {
        Discrepancy::MissingFile { file_path, package_name, package_version, .. } |
        Discrepancy::CorruptedFile { file_path, package_name, package_version, .. } => (
            file_path.clone(),
            Some(package_name.clone()),
            Some(package_version.to_string()),
        ),
        Discrepancy::MissingVenv { venv_path, package_name, package_version } => (
            venv_path.clone(),
            Some(package_name.clone()),
            Some(package_version.to_string()),
        ),
        Discrepancy::OrphanedFile { file_path, .. } => (
            file_path.clone(),
            None,
            None,
        ),
        Discrepancy::TypeMismatch { file_path, package_name, package_version, .. } => (
            file_path.clone(),
            Some(package_name.clone()),
            Some(package_version.to_string()),
        ),
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

/// Check if a path should be handled with lenient symlink policy
fn should_be_lenient_with_symlinks(path: &Path, lenient_directories: &[PathBuf]) -> bool {
    for lenient_dir in lenient_directories {
        if path.starts_with(lenient_dir) {
            return true;
        }
    }
    false
}

/// Handle symlink verification based on policy
async fn handle_symlink_verification(
    full_path: &Path,
    relative_path: &str,
    package: &sps2_state::models::Package,
    symlink_policy: SymlinkPolicy,
    lenient_directories: &[PathBuf],
    discrepancies: &mut Vec<Discrepancy>,
    operation_id: &str,
    tx: Option<&sps2_events::EventSender>,
) -> Result<bool, Error> {
    // Check if path exists and is a symlink
    if !full_path.exists() {
        return Ok(false); // Not a symlink issue, will be handled as missing file
    }

    let metadata = tokio::fs::symlink_metadata(full_path).await?;
    if !metadata.is_symlink() {
        return Ok(false); // Not a symlink, proceed with normal verification
    }

    match symlink_policy {
        SymlinkPolicy::Ignore => {
            // Skip symlink verification entirely
            if let Some(sender) = tx {
                let _ = sender.send(Event::DebugLog {
                    message: format!("Ignoring symlink verification: {relative_path}"),
                    context: HashMap::default(),
                });
            }
            return Ok(true); // Skip this file
        }
        SymlinkPolicy::Lenient => {
            // Check if this path should be handled leniently
            if should_be_lenient_with_symlinks(full_path, lenient_directories) {
                if let Some(sender) = tx {
                    let _ = sender.send(Event::DebugLog {
                        message: format!(
                            "Lenient symlink handling for bootstrap directory: {relative_path}"
                        ),
                        context: HashMap::default(),
                    });
                }
                return Ok(true); // Skip verification but log it
            }
            // Not in lenient directory, proceed with strict verification
        }
        SymlinkPolicy::Strict => {
            // Proceed with strict symlink verification
        }
    }

    // For strict verification or lenient files outside protected directories,
    // check if the symlink target exists
    match tokio::fs::read_link(full_path).await {
        Ok(target) => {
            let absolute_target = if target.is_absolute() {
                target
            } else {
                full_path.parent().unwrap_or(full_path).join(target)
            };

            if !absolute_target.exists() {
                // Symlink points to non-existent target
                if symlink_policy == SymlinkPolicy::Lenient && should_be_lenient_with_symlinks(full_path, lenient_directories) {
                    if let Some(sender) = tx {
                        let _ = sender.send(Event::DebugLog {
                            message: format!(
                                "Broken symlink in lenient directory (not failing): {relative_path} -> {}",
                                absolute_target.display()
                            ),
                            context: HashMap::default(),
                        });
                    }
                    return Ok(true); // Don't fail, just log
                } else {
                    // Add discrepancy for broken symlink
                    add_discrepancy_with_event(
                        discrepancies,
                        Discrepancy::CorruptedFile {
                            package_name: package.name.clone(),
                            package_version: package.version.clone(),
                            file_path: relative_path.to_string(),
                            expected_hash: "valid_symlink".to_string(),
                            actual_hash: "broken_symlink".to_string(),
                        },
                        operation_id,
                        tx,
                    );
                    return Ok(true); // Handled as discrepancy
                }
            }
        }
        Err(_) => {
            // Error reading symlink
            if symlink_policy == SymlinkPolicy::Lenient && should_be_lenient_with_symlinks(full_path, lenient_directories) {
                if let Some(sender) = tx {
                    let _ = sender.send(Event::DebugLog {
                        message: format!("Symlink read error in lenient directory (not failing): {relative_path}"),
                        context: HashMap::default(),
                    });
                }
                return Ok(true); // Don't fail, just log
            } else {
                return Err(sps2_errors::OpsError::OperationFailed {
                    message: format!("Failed to read symlink: {relative_path}"),
                }.into());
            }
        }
    }

    Ok(false) // Proceed with normal verification
}

/// Verify a single package
pub async fn verify_package(
    ctx: &mut VerificationContext<'_>,
    package: &sps2_state::models::Package,
    discrepancies: &mut Vec<Discrepancy>,
    tracked_files: &mut HashSet<PathBuf>,
    operation_id: &str,
) -> Result<(), Error> {
    // Get package files from database
    let mut tx = ctx.state_manager.begin_transaction().await?;
    let file_paths =
        queries::get_package_files(&mut tx, ctx.state_id, &package.name, &package.version).await?;
    tx.commit().await?;

    // Verify each file
    for file_path in file_paths {
        let full_path = ctx.live_path.join(&file_path);
        tracked_files.insert(PathBuf::from(&file_path));

        // Check cache first
        if ctx.cache.is_entry_valid(&file_path, ctx.level) {
            // Cache hit - use cached result
            if let Some(cached_entry) = ctx.cache.get_entry(&file_path) {
                if !cached_entry.was_valid {
                    // Cached entry indicates previous failure, add to discrepancies
                    if !full_path.exists() {
                        add_discrepancy_with_event(
                            discrepancies,
                            Discrepancy::MissingFile {
                                package_name: package.name.clone(),
                                package_version: package.version.clone(),
                                file_path: file_path.clone(),
                            },
                            operation_id,
                            ctx.tx,
                        );
                    }
                }
                // Skip verification for cached files
                continue;
            }
        }

        // Cache miss - perform verification
        let _verification_start = Instant::now();
        let mut file_was_valid = true;

        // Check if file exists
        if !full_path.exists() {
            add_discrepancy_with_event(
                discrepancies,
                Discrepancy::MissingFile {
                    package_name: package.name.clone(),
                    package_version: package.version.clone(),
                    file_path: file_path.clone(),
                },
                operation_id,
                ctx.tx,
            );
            file_was_valid = false;
        } else {
            // Handle symlink verification based on policy
            if handle_symlink_verification(
                &full_path,
                &file_path,
                package,
                ctx.guard_config.symlink_policy,
                &ctx.guard_config.lenient_symlink_directories,
                discrepancies,
                operation_id,
                ctx.tx,
            )
            .await?
            {
                // Symlink was handled (skipped or logged), continue to next file
                continue;
            }

            // For Full verification, check content hash
            if ctx.level == crate::types::VerificationLevel::Full {
                let discrepancy_count_before = discrepancies.len();
                verify_file_content(ctx.store, &full_path, package, &file_path, discrepancies, operation_id, ctx.tx)
                    .await?;
                // If discrepancies were added, file was invalid
                if discrepancies.len() > discrepancy_count_before {
                    file_was_valid = false;
                }
            }
        }

        // Update cache with verification result
        if let Ok(metadata) = std::fs::metadata(&full_path) {
            if let Ok(mtime) = metadata.modified() {
                let size = metadata.len();
                let content_hash =
                    if ctx.level == crate::types::VerificationLevel::Full && file_was_valid {
                        // Calculate hash for Full verification
                        match Hash::hash_file(&full_path).await {
                            Ok(hash) => Some(hash.to_string()),
                            Err(_) => None,
                        }
                    } else {
                        None
                    };

                let cache_entry = FileCacheEntry {
                    file_path: file_path.clone(),
                    package_name: package.name.clone(),
                    package_version: package.version.clone(),
                    mtime,
                    size,
                    content_hash,
                    verified_at: SystemTime::now(),
                    verification_level: ctx.level,
                    was_valid: file_was_valid,
                };

                ctx.cache.update_entry(cache_entry);
            }
        }
    }

    // Check Python venv if applicable
    if let Some(venv_path) = &package.venv_path {
        if !Path::new(venv_path).exists() {
            add_discrepancy_with_event(
                discrepancies,
                Discrepancy::MissingVenv {
                    package_name: package.name.clone(),
                    package_version: package.version.clone(),
                    venv_path: venv_path.clone(),
                },
                operation_id,
                ctx.tx,
            );
        }
    }

    Ok(())
}
