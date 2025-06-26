//! Package verification logic

use crate::types::{Discrepancy, FileCacheEntry, VerificationContext};
use crate::verification::content::verify_file_content;
use sps2_errors::Error;
use sps2_hash::Hash;
use sps2_state::queries;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

/// Verify a single package
pub async fn verify_package(
    ctx: &mut VerificationContext<'_>,
    package: &sps2_state::models::Package,
    discrepancies: &mut Vec<Discrepancy>,
    tracked_files: &mut HashSet<PathBuf>,
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
                        discrepancies.push(Discrepancy::MissingFile {
                            package_name: package.name.clone(),
                            package_version: package.version.clone(),
                            file_path: file_path.clone(),
                        });
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
            discrepancies.push(Discrepancy::MissingFile {
                package_name: package.name.clone(),
                package_version: package.version.clone(),
                file_path: file_path.clone(),
            });
            file_was_valid = false;
        } else {
            // For Full verification, check content hash
            if ctx.level == crate::types::VerificationLevel::Full {
                let discrepancy_count_before = discrepancies.len();
                verify_file_content(ctx.store, &full_path, package, &file_path, discrepancies)
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
            discrepancies.push(Discrepancy::MissingVenv {
                package_name: package.name.clone(),
                package_version: package.version.clone(),
                venv_path: venv_path.clone(),
            });
        }
    }

    Ok(())
}
