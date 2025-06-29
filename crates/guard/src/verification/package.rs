//! Package verification logic

use crate::types::{Discrepancy, SymlinkPolicy, VerificationContext};
use crate::verification::content::{verify_file_content, ContentVerificationParams};
use sps2_errors::Error;
use sps2_events::Event;
use sps2_state::queries;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Helper function to add a discrepancy and emit the corresponding event
fn add_discrepancy_with_event(
    discrepancies: &mut Vec<Discrepancy>,
    discrepancy: Discrepancy,
    operation_id: &str,
    tx: Option<&sps2_events::EventSender>,
) {
    // Determine severity and user message based on discrepancy type
    let (severity, user_message, auto_heal_available) = match &discrepancy {
        Discrepancy::MissingFile { .. } => ("high", "File is missing from the filesystem", true),
        Discrepancy::CorruptedFile { .. } => {
            ("high", "File content does not match expected hash", true)
        }
        Discrepancy::MissingVenv { .. } => {
            ("medium", "Python virtual environment is missing", true)
        }
        Discrepancy::OrphanedFile { .. } => (
            "low",
            "File exists but is not tracked by any package",
            false,
        ),
        Discrepancy::TypeMismatch { .. } => {
            ("medium", "File type does not match expected type", true)
        }
    };

    // Extract file path and package info
    let (file_path, package, package_version) = match &discrepancy {
        Discrepancy::MissingFile {
            file_path,
            package_name,
            package_version,
            ..
        }
        | Discrepancy::CorruptedFile {
            file_path,
            package_name,
            package_version,
            ..
        } => (
            file_path.clone(),
            Some(package_name.clone()),
            Some(package_version.to_string()),
        ),
        Discrepancy::MissingVenv {
            venv_path,
            package_name,
            package_version,
        } => (
            venv_path.clone(),
            Some(package_name.clone()),
            Some(package_version.to_string()),
        ),
        Discrepancy::OrphanedFile { file_path, .. } => (file_path.clone(), None, None),
        Discrepancy::TypeMismatch {
            file_path,
            package_name,
            package_version,
            ..
        } => (
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

/// Parameters for symlink verification
struct SymlinkVerificationParams<'a> {
    symlink_policy: SymlinkPolicy,
    lenient_directories: &'a [PathBuf],
    operation_id: &'a str,
    tx: Option<&'a sps2_events::EventSender>,
}

/// Handle symlink verification based on policy
async fn handle_symlink_verification(
    full_path: &Path,
    relative_path: &str,
    package: &sps2_state::models::Package,
    discrepancies: &mut Vec<Discrepancy>,
    params: SymlinkVerificationParams<'_>,
) -> Result<bool, Error> {
    // Check if path exists and is a symlink
    if !full_path.exists() {
        return Ok(false); // Not a symlink issue, will be handled as missing file
    }

    let metadata = tokio::fs::symlink_metadata(full_path).await?;
    if !metadata.is_symlink() {
        return Ok(false); // Not a symlink, proceed with normal verification
    }

    match params.symlink_policy {
        SymlinkPolicy::Ignore => {
            // Skip symlink verification entirely
            if let Some(sender) = params.tx {
                let _ = sender.send(Event::DebugLog {
                    message: format!("Ignoring symlink verification: {relative_path}"),
                    context: HashMap::default(),
                });
            }
            return Ok(true); // Skip this file
        }
        SymlinkPolicy::Lenient => {
            // Check if this path should be handled leniently
            if should_be_lenient_with_symlinks(full_path, params.lenient_directories) {
                if let Some(sender) = params.tx {
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
                if params.symlink_policy == SymlinkPolicy::Lenient
                    && should_be_lenient_with_symlinks(full_path, params.lenient_directories)
                {
                    if let Some(sender) = params.tx {
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
                        params.operation_id,
                        params.tx,
                    );
                    return Ok(true); // Handled as discrepancy
                }
            }
        }
        Err(_) => {
            // Error reading symlink
            if params.symlink_policy == SymlinkPolicy::Lenient
                && should_be_lenient_with_symlinks(full_path, params.lenient_directories)
            {
                if let Some(sender) = params.tx {
                    let _ = sender.send(Event::DebugLog {
                        message: format!("Symlink read error in lenient directory (not failing): {relative_path}"),
                        context: HashMap::default(),
                    });
                }
                return Ok(true); // Don't fail, just log
            } else {
                return Err(sps2_errors::OpsError::OperationFailed {
                    message: format!("Failed to read symlink: {relative_path}"),
                }
                .into());
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

    // Debug: log how many files we're checking
    if let Some(sender) = ctx.tx {
        let _ = sender.send(Event::DebugLog {
            message: format!(
                "Verifying {} files for package {}-{}",
                file_paths.len(),
                package.name,
                package.version
            ),
            context: HashMap::default(),
        });
    }

    // Verify each file
    for file_path in file_paths {
        // Strip the legacy prefix if present (packages store paths like "opt/pm/live/bin/foo")
        let clean_path = if let Some(stripped) = file_path.strip_prefix("opt/pm/live/") {
            stripped
        } else if file_path == "opt" || file_path == "opt/pm" || file_path == "opt/pm/live" {
            // Skip these directory entries entirely - they're artifacts of the legacy format
            continue;
        } else {
            &file_path
        };

        let full_path = ctx.live_path.join(clean_path);

        // Check if this file should be verified based on scope
        if !crate::verification::scope::should_verify_file(&full_path, ctx.scope) {
            continue;
        }

        tracked_files.insert(PathBuf::from(clean_path));

        // Debug: log the path we're checking
        if package.name == "bat" {
            if let Some(sender) = ctx.tx {
                let _ = sender.send(Event::DebugLog {
                    message: format!(
                        "Checking bat file: {} -> {}",
                        file_path,
                        full_path.display()
                    ),
                    context: HashMap::default(),
                });
            }
        }

        // Perform verification
        let _verification_start = Instant::now();

        // Check if file exists
        if !full_path.exists() {
            add_discrepancy_with_event(
                discrepancies,
                Discrepancy::MissingFile {
                    package_name: package.name.clone(),
                    package_version: package.version.clone(),
                    file_path: clean_path.to_string(),
                },
                operation_id,
                ctx.tx,
            );
        } else {
            // Handle symlink verification based on policy
            if handle_symlink_verification(
                &full_path,
                clean_path,
                package,
                discrepancies,
                SymlinkVerificationParams {
                    symlink_policy: ctx.guard_config.symlink_policy,
                    lenient_directories: &ctx.guard_config.lenient_symlink_directories,
                    operation_id,
                    tx: ctx.tx,
                },
            )
            .await?
            {
                // Symlink was handled (skipped or logged), continue to next file
                continue;
            }

            // For Full verification, check content hash
            if ctx.level == crate::types::VerificationLevel::Full {
                // Debug: log content verification
                if package.name == "bat" && clean_path == "bin/bat" {
                    if let Some(sender) = ctx.tx {
                        let _ = sender.send(Event::DebugLog {
                            message: format!(
                                "Running content verification for bat binary at {}",
                                full_path.display()
                            ),
                            context: HashMap::default(),
                        });
                    }
                }

                verify_file_content(ContentVerificationParams {
                    state_manager: ctx.state_manager,
                    state_id: ctx.state_id,
                    file_path: &full_path,
                    package,
                    relative_path: clean_path,
                    discrepancies,
                    operation_id,
                    tx: ctx.tx,
                })
                .await?;
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
