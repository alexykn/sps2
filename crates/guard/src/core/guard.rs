//! Main StateVerificationGuard implementation

use crate::error_context::{GuardErrorContext, VerbosityLevel};
use crate::types::{
    Discrepancy, GuardConfig, HealingContext, OperationType, VerificationLevel, VerificationResult,
    VerificationScope,
};
use crate::verification;
use sps2_errors::Error;
use sps2_events::{Event, EventSender};
use sps2_hash::Hash;
use sps2_state::{queries, PackageFileEntry, StateManager};
use sps2_store::PackageStore;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use uuid;

/// Result of verifying a single package
#[derive(Debug)]
struct SinglePackageResult {
    discrepancies: Vec<Discrepancy>,
    tracked_files: HashSet<std::path::PathBuf>,
    mtime_updates: Vec<MTimeUpdate>,
    cache_hits: usize,
    cache_misses: usize,
}

/// MTime update to be applied after parallel verification
#[derive(Debug, Clone)]
struct MTimeUpdate {
    file_path: String,
    verified_mtime: i64,
}

/// Pre-fetched data for a package
#[derive(Debug, Clone)]
struct PackageData {
    package: sps2_state::Package,
    file_entries: Vec<PackageFileEntry>,
    mtime_trackers: HashMap<String, i64>, // file_path -> last_verified_mtime
}

/// Verify a single package with pre-fetched data (for parallel verification)
async fn verify_single_package_with_data(
    _state_manager: &StateManager,
    store: &PackageStore,
    package_data: PackageData,
    level: VerificationLevel,
    _guard_config: &GuardConfig,
    live_path: &std::path::Path,
    _state_id: &uuid::Uuid,
) -> Result<(String, String, SinglePackageResult), Error> {
    let package = &package_data.package;
    let file_entries = &package_data.file_entries;
    let mut discrepancies = Vec::new();
    let mut tracked_files: HashSet<std::path::PathBuf> = HashSet::new();
    let mut mtime_updates = Vec::new();
    let mut cache_hits = 0;
    let mut cache_misses = 0;

    // Get package manifest from store
    let package_hash =
        Hash::from_hex(&package.hash).map_err(|e| sps2_errors::OpsError::OperationFailed {
            message: format!("Invalid package hash: {e}"),
        })?;
    let store_path = store.package_path(&package_hash);

    if !store_path.exists() {
        // Package content missing - can't verify files
        discrepancies.push(Discrepancy::MissingPackageContent {
            package_name: package.name.clone(),
            package_version: package.version.clone(),
        });
        return Ok((
            package.name.clone(),
            package.version.clone(),
            SinglePackageResult {
                discrepancies,
                tracked_files,
                mtime_updates,
                cache_hits,
                cache_misses,
            },
        ));
    }

    // Verify package exists in store (but we already have file entries from pre-fetch)
    let _ = sps2_store::StoredPackage::load(&store_path).await?;

    // Process all files from pre-fetched file entries to ensure they're all tracked
    for entry in file_entries {
        let file_path = &entry.relative_path;
        // Strip legacy prefix if present
        let clean_path = if let Some(stripped) = file_path.strip_prefix("opt/pm/live/") {
            stripped
        } else if file_path == "opt" || file_path == "opt/pm" || file_path == "opt/pm/live" {
            continue;
        } else {
            file_path
        };

        tracked_files.insert(std::path::PathBuf::from(clean_path));
        let full_path = live_path.join(clean_path);

        // Basic existence check
        if !full_path.exists() {
            discrepancies.push(Discrepancy::MissingFile {
                package_name: package.name.clone(),
                package_version: package.version.clone(),
                file_path: clean_path.to_string(),
            });
            continue;
        }

        // For Full verification, check content hash
        if level == VerificationLevel::Full {
            // Skip hash verification for directories and symlinks
            let metadata = tokio::fs::symlink_metadata(&full_path).await?;
            if metadata.is_dir() || metadata.is_symlink() {
                continue;
            }

            // Find the file entry for this path
            let file_entry = file_entries
                .iter()
                .find(|entry| entry.relative_path == clean_path);

            if let Some(entry) = file_entry {
                let expected_hash = Hash::from_hex(&entry.file_hash).map_err(|e| {
                    sps2_errors::OpsError::OperationFailed {
                        message: format!("Invalid file hash in database: {e}"),
                    }
                })?;

                // MTIME-ONLY OPTIMIZATION: Only verify if file has been modified
                let needs_verification = {
                    // Get current file modification time
                    let file_mtime = metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);

                    // Check if we have a stored mtime for this file
                    if let Some(&last_verified_mtime) = package_data.mtime_trackers.get(clean_path)
                    {
                        if file_mtime > last_verified_mtime {
                            // File has been modified since last verification
                            cache_misses += 1;
                            true
                        } else {
                            // File unchanged since last verification - skip
                            cache_hits += 1;
                            false
                        }
                    } else {
                        // No mtime stored - need to verify
                        cache_misses += 1;
                        true
                    }
                };

                if needs_verification {
                    // Get current file mtime for tracking
                    let file_mtime = metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);

                    // Calculate actual hash
                    let actual_hash = Hash::hash_file(&full_path).await?;

                    if actual_hash != expected_hash {
                        discrepancies.push(Discrepancy::CorruptedFile {
                            package_name: package.name.clone(),
                            package_version: package.version.clone(),
                            file_path: clean_path.to_string(),
                            expected_hash: expected_hash.to_hex(),
                            actual_hash: actual_hash.to_hex(),
                        });
                    }

                    // Always update mtime tracker after verification (success or failure)
                    mtime_updates.push(MTimeUpdate {
                        file_path: clean_path.to_string(),
                        verified_mtime: file_mtime,
                    });
                }
            }
        }
    }

    // Check Python venv if applicable
    if let Some(venv_path) = &package.venv_path {
        if !std::path::Path::new(venv_path).exists() {
            discrepancies.push(Discrepancy::MissingVenv {
                package_name: package.name.clone(),
                package_version: package.version.clone(),
                venv_path: venv_path.clone(),
            });
        }
    }

    Ok((
        package.name.clone(),
        package.version.clone(),
        SinglePackageResult {
            discrepancies,
            tracked_files,
            mtime_updates,
            cache_hits,
            cache_misses,
        },
    ))
}

/// State verification guard for consistency checking
pub struct StateVerificationGuard {
    /// State manager for database operations
    state_manager: StateManager,
    /// Package store for content verification
    store: PackageStore,
    /// Event sender for progress reporting
    tx: EventSender,
    /// Guard configuration including verification level, policies, and performance settings
    config: GuardConfig,
}

impl StateVerificationGuard {
    /// Create a new StateVerificationGuard
    ///
    /// This is used internally by the builder pattern.
    #[must_use]
    pub(crate) fn new(
        state_manager: StateManager,
        store: PackageStore,
        tx: EventSender,
        config: GuardConfig,
    ) -> Self {
        Self {
            state_manager,
            store,
            tx,
            config,
        }
    }

    /// Create a new verification guard with builder
    #[must_use]
    pub fn builder() -> crate::core::StateVerificationGuardBuilder {
        crate::core::StateVerificationGuardBuilder::new()
    }

    /// Get the current verification level
    #[must_use]
    pub const fn level(&self) -> VerificationLevel {
        self.config.verification_level
    }

    /// Get the guard configuration
    #[must_use]
    pub const fn config(&self) -> &GuardConfig {
        &self.config
    }

    /// Verify current state without healing
    ///
    /// # Errors
    ///
    /// Returns an error if state verification fails or database operations fail.
    pub async fn verify_only(&mut self) -> Result<VerificationResult, Error> {
        let state_id = self.state_manager.get_active_state().await?;

        // Get all installed packages from current state
        let mut tx = self.state_manager.begin_transaction().await?;
        let packages = queries::get_state_packages(&mut tx, &state_id).await?;
        tx.commit().await?;

        // Create error context for comprehensive event emission
        let mut error_ctx = GuardErrorContext::for_verification(
            OperationType::Verify {
                scope: VerificationScope::Full,
            },
            self.tx.clone(),
            VerbosityLevel::Standard,
        );

        // Emit verification started event
        error_ctx.emit_operation_start(
            "system",
            &format!("{:?}", self.config.verification_level),
            packages.len(),
            None,
        );

        // Note: verify_packages_parallel handles orphan detection internally
        // when verification level is not Quick
        let result = self
            .verify_packages_parallel(&packages, &VerificationScope::Full)
            .await;

        // Emit completion or failure events
        match &result {
            Ok(verification_result) => {
                error_ctx.record_verification_result(verification_result);
                let coverage_percent = verification_result
                    .coverage
                    .as_ref()
                    .map(|c| c.package_coverage_percent)
                    .unwrap_or(100.0);
                error_ctx.emit_operation_completed(
                    verification_result.cache_hit_rate,
                    coverage_percent,
                    "system verification",
                );
                error_ctx.emit_error_summary();
            }
            Err(_) => {
                let _ = self.tx.send(Event::GuardVerificationFailed {
                    operation_id: error_ctx.operation_id().to_string(),
                    error: "Verification operation failed".to_string(),
                    packages_verified: 0,
                    files_verified: 0,
                    duration_ms: 0,
                });
            }
        }

        result
    }

    /// Verify current state with specific scope without healing
    ///
    /// # Errors
    ///
    /// Returns an error if state verification fails or database operations fail.
    pub async fn verify_with_scope(
        &mut self,
        scope: &VerificationScope,
    ) -> Result<VerificationResult, Error> {
        let state_id = self.state_manager.get_active_state().await?;

        // Emit verification started event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Starting scoped state verification for state {state_id} (scope: {:?})",
                scope
            ),
            context: HashMap::default(),
        });

        // Get packages based on scope
        let (packages_to_verify, total_packages, total_files) =
            verification::scope::get_packages_for_scope(&self.state_manager, &state_id, scope)
                .await?;

        // Always use parallel verification (better performance with batched DB writes)
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Using parallel verification for {} packages in scope {:?} (max concurrent: {})",
                packages_to_verify.len(),
                scope,
                self.config.performance.max_concurrent_tasks
            ),
            context: HashMap::default(),
        });

        // Use parallel verification
        let mut result = self
            .verify_packages_parallel(&packages_to_verify, scope)
            .await?;

        // Update coverage with scope-specific totals
        if let Some(ref mut coverage) = result.coverage {
            coverage.total_packages = total_packages;
            coverage.total_files = total_files;
            coverage.package_coverage_percent = if total_packages > 0 {
                (coverage.verified_packages as f64 / total_packages as f64) * 100.0
            } else {
                100.0
            };
            coverage.file_coverage_percent = if total_files > 0 {
                (coverage.verified_files as f64 / total_files as f64) * 100.0
            } else {
                100.0
            };
        }

        Ok(result)
    }

    /// Verify current state and optionally heal discrepancies
    ///
    /// # Errors
    ///
    /// Returns an error if state verification fails or database operations fail.
    pub async fn verify_and_heal(
        &mut self,
        config: &sps2_config::Config,
    ) -> Result<VerificationResult, Error> {
        // First, run verification to detect discrepancies
        let mut verification_result = self.verify_only().await?;

        // If no discrepancies found, return early
        if verification_result.is_valid {
            return Ok(verification_result);
        }

        // Create healing error context
        let mut healing_ctx_events = GuardErrorContext::for_healing(
            OperationType::Verify {
                scope: VerificationScope::Full,
            },
            self.tx.clone(),
            VerbosityLevel::Standard,
        );

        // Emit healing start event
        let auto_heal_count = verification_result
            .discrepancies
            .iter()
            .filter(|d| d.can_auto_heal())
            .count();
        let confirmation_required = verification_result
            .discrepancies
            .iter()
            .filter(|d| d.requires_confirmation())
            .count();
        let manual_intervention_count =
            verification_result.discrepancies.len() - auto_heal_count - confirmation_required;

        let _ = self.tx.send(Event::GuardHealingStarted {
            operation_id: healing_ctx_events.operation_id().to_string(),
            discrepancies_count: verification_result.discrepancies.len(),
            auto_heal_count,
            confirmation_required_count: confirmation_required,
            manual_intervention_count,
        });

        // Create healing context
        let healing_ctx = HealingContext {
            state_manager: &self.state_manager,
            store: &self.store,
            tx: &self.tx,
        };

        // Track healing results
        let mut healed_count = 0;
        let mut failed_healings = Vec::new();

        // Process each discrepancy
        let discrepancies = verification_result.discrepancies.clone();
        for (idx, discrepancy) in discrepancies.iter().enumerate() {
            // Emit healing progress
            let _ = self.tx.send(Event::GuardHealingProgress {
                operation_id: healing_ctx_events.operation_id().to_string(),
                completed: idx,
                total: discrepancies.len(),
                current_operation: discrepancy.short_description().to_string(),
                current_file: Some(discrepancy.file_path().to_string()),
            });

            match discrepancy {
                Discrepancy::MissingFile {
                    package_name,
                    package_version,
                    file_path,
                } => {
                    match crate::healing::files::restore_missing_file(
                        &healing_ctx,
                        package_name,
                        package_version,
                        file_path,
                    )
                    .await
                    {
                        Ok(()) => {
                            healed_count += 1;
                            healing_ctx_events.emit_healing_result(
                                "MissingFile",
                                file_path,
                                true,
                                "file restored from store",
                                None,
                            );
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            healing_ctx_events.emit_healing_result(
                                "MissingFile",
                                file_path,
                                false,
                                "file restoration failed",
                                Some(e.to_string()),
                            );
                        }
                    }
                }
                Discrepancy::OrphanedFile {
                    file_path,
                    category,
                } => {
                    match crate::healing::orphans::handle_orphaned_file(
                        &self.state_manager,
                        &self.tx,
                        file_path,
                        category,
                        config,
                    )
                    .await
                    {
                        Ok(()) => {
                            healed_count += 1;
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Successfully handled orphaned file: {file_path} (category: {category:?})"),
                                context: HashMap::default(),
                            });
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Failed to handle orphaned file {file_path}: {e}"),
                                context: HashMap::default(),
                            });
                        }
                    }
                }
                Discrepancy::CorruptedFile {
                    package_name,
                    package_version,
                    file_path,
                    expected_hash,
                    actual_hash,
                } => {
                    match crate::healing::files::heal_corrupted_file(
                        &healing_ctx,
                        package_name,
                        package_version,
                        file_path,
                        expected_hash,
                        actual_hash,
                    )
                    .await
                    {
                        Ok(()) => {
                            healed_count += 1;
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Successfully restored corrupted file: {file_path} for {package_name}-{package_version}"),
                                context: HashMap::default(),
                            });
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!(
                                    "Failed to restore corrupted file {file_path}: {e}"
                                ),
                                context: HashMap::default(),
                            });
                        }
                    }
                }
                Discrepancy::MissingVenv {
                    package_name,
                    package_version,
                    venv_path,
                } => {
                    match crate::healing::venv::heal_missing_venv(
                        &self.state_manager,
                        &self.store,
                        &self.tx,
                        package_name,
                        package_version,
                        venv_path,
                    )
                    .await
                    {
                        Ok(()) => {
                            healed_count += 1;
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Successfully healed missing venv: {venv_path} for {package_name}-{package_version}"),
                                context: HashMap::default(),
                            });
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Failed to heal missing venv {venv_path}: {e}"),
                                context: HashMap::default(),
                            });
                        }
                    }
                }
                // Handle other discrepancy types as needed
                _ => {
                    failed_healings.push(discrepancy.clone());
                }
            }
        }

        // Update verification result with healing results
        verification_result.discrepancies = failed_healings.clone();
        verification_result.is_valid = verification_result.discrepancies.is_empty();

        let duration_ms = u64::try_from(
            healing_ctx_events
                .get_summary_stats()
                .operation_duration
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        verification_result.duration_ms = duration_ms;

        // Emit healing completion event
        let _ = self.tx.send(Event::GuardHealingCompleted {
            operation_id: healing_ctx_events.operation_id().to_string(),
            healed_count,
            failed_count: failed_healings.len(),
            skipped_count: 0,
            duration_ms,
        });

        // Record healing results in context and emit summary
        for discrepancy in &failed_healings {
            healing_ctx_events.record_discrepancy(discrepancy.clone());
        }
        healing_ctx_events.emit_error_summary();

        Ok(verification_result)
    }

    /// Verify current state with specific scope and optionally heal discrepancies
    ///
    /// # Errors
    ///
    /// Returns an error if state verification fails or database operations fail.
    pub async fn verify_and_heal_scoped(
        &mut self,
        config: &sps2_config::Config,
        scope: &VerificationScope,
    ) -> Result<VerificationResult, Error> {
        let start_time = Instant::now();

        // First, run verification to detect discrepancies
        let mut verification_result = self.verify_with_scope(scope).await?;

        // If no discrepancies found, return early
        if verification_result.is_valid {
            return Ok(verification_result);
        }

        // Create healing error context for events
        let healing_ctx_events = GuardErrorContext::for_healing(
            OperationType::Verify {
                scope: scope.clone(),
            },
            self.tx.clone(),
            VerbosityLevel::Standard,
        );

        // Emit healing start event
        let auto_heal_count = verification_result
            .discrepancies
            .iter()
            .filter(|d| d.can_auto_heal())
            .count();
        let confirmation_required = verification_result
            .discrepancies
            .iter()
            .filter(|d| d.requires_confirmation())
            .count();
        let manual_intervention_count =
            verification_result.discrepancies.len() - auto_heal_count - confirmation_required;

        let _ = self.tx.send(Event::GuardHealingStarted {
            operation_id: healing_ctx_events.operation_id().to_string(),
            discrepancies_count: verification_result.discrepancies.len(),
            auto_heal_count,
            confirmation_required_count: confirmation_required,
            manual_intervention_count,
        });

        // Create healing context
        let healing_ctx = HealingContext {
            state_manager: &self.state_manager,
            store: &self.store,
            tx: &self.tx,
        };

        // Track healing results - use same healing logic as full verification
        let mut healed_count = 0;
        let mut failed_healings = Vec::new();

        let discrepancies = verification_result.discrepancies.clone();
        for discrepancy in &discrepancies {
            match discrepancy {
                Discrepancy::MissingFile {
                    package_name,
                    package_version,
                    file_path,
                } => {
                    match crate::healing::files::restore_missing_file(
                        &healing_ctx,
                        package_name,
                        package_version,
                        file_path,
                    )
                    .await
                    {
                        Ok(()) => {
                            healed_count += 1;
                            healing_ctx_events.emit_healing_result(
                                "MissingFile",
                                file_path,
                                true,
                                "file restored from store",
                                None,
                            );
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            healing_ctx_events.emit_healing_result(
                                "MissingFile",
                                file_path,
                                false,
                                "file restoration failed",
                                Some(e.to_string()),
                            );
                        }
                    }
                }
                Discrepancy::OrphanedFile {
                    file_path,
                    category,
                } => {
                    match crate::healing::orphans::handle_orphaned_file(
                        &self.state_manager,
                        &self.tx,
                        file_path,
                        category,
                        config,
                    )
                    .await
                    {
                        Ok(()) => {
                            healed_count += 1;
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Successfully handled orphaned file: {file_path} (category: {category:?})"),
                                context: HashMap::default(),
                            });
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Failed to handle orphaned file {file_path}: {e}"),
                                context: HashMap::default(),
                            });
                        }
                    }
                }
                Discrepancy::CorruptedFile {
                    package_name,
                    package_version,
                    file_path,
                    expected_hash,
                    actual_hash,
                } => {
                    match crate::healing::files::heal_corrupted_file(
                        &healing_ctx,
                        package_name,
                        package_version,
                        file_path,
                        expected_hash,
                        actual_hash,
                    )
                    .await
                    {
                        Ok(()) => {
                            healed_count += 1;
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Successfully restored corrupted file: {file_path} for {package_name}-{package_version}"),
                                context: HashMap::default(),
                            });
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!(
                                    "Failed to restore corrupted file {file_path}: {e}"
                                ),
                                context: HashMap::default(),
                            });
                        }
                    }
                }
                Discrepancy::MissingVenv {
                    package_name,
                    package_version,
                    venv_path,
                } => {
                    match crate::healing::venv::heal_missing_venv(
                        &self.state_manager,
                        &self.store,
                        &self.tx,
                        package_name,
                        package_version,
                        venv_path,
                    )
                    .await
                    {
                        Ok(()) => {
                            healed_count += 1;
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Successfully healed missing venv: {venv_path} for {package_name}-{package_version}"),
                                context: HashMap::default(),
                            });
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Failed to heal missing venv {venv_path}: {e}"),
                                context: HashMap::default(),
                            });
                        }
                    }
                }
                _ => {
                    failed_healings.push(discrepancy.clone());
                }
            }
        }

        // Update verification result with healing results
        verification_result.discrepancies = failed_healings;
        verification_result.is_valid = verification_result.discrepancies.is_empty();

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
        verification_result.duration_ms = duration_ms;

        // Emit healing complete event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Scoped healing completed: {} healed, {} failed in {}ms",
                healed_count,
                verification_result.discrepancies.len(),
                duration_ms
            ),
            context: HashMap::default(),
        });

        Ok(verification_result)
    }

    /// Progressive verification with automatic escalation
    ///
    /// Starts with Quick verification and escalates to higher levels only when issues are found.
    /// This provides optimal performance while maintaining verification accuracy.
    ///
    /// # Errors
    ///
    /// Returns an error if verification fails.
    pub async fn verify_progressively(
        &mut self,
        scope: &VerificationScope,
    ) -> Result<VerificationResult, Error> {
        if !self.config.performance.progressive_verification {
            // Progressive verification disabled - use configured level
            return self.verify_with_scope(scope).await;
        }

        let start_time = Instant::now();
        let _state_id = self.state_manager.get_active_state().await?;

        // Stage 1: Quick verification
        let _ = self.tx.send(Event::DebugLog {
            message: "Progressive verification: Starting with Quick level".to_string(),
            context: HashMap::default(),
        });

        let original_level = self.config.verification_level;
        self.config.verification_level = VerificationLevel::Quick;
        let quick_result = self.verify_with_scope(scope).await?;
        self.config.verification_level = original_level;

        if quick_result.is_valid {
            // No issues found - we're done!
            let _ = self.tx.send(Event::DebugLog {
                message:
                    "Progressive verification: Quick verification passed, no escalation needed"
                        .to_string(),
                context: HashMap::default(),
            });
            return Ok(quick_result);
        }

        // Stage 2: Standard verification (if issues found and original level >= Standard)
        if original_level >= VerificationLevel::Standard {
            let _ = self.tx.send(Event::DebugLog {
                message: format!(
                    "Progressive verification: Quick found {} issues, escalating to Standard",
                    quick_result.discrepancies.len()
                ),
                context: HashMap::default(),
            });

            self.config.verification_level = VerificationLevel::Standard;
            let standard_result = self.verify_with_scope(scope).await?;
            self.config.verification_level = original_level;

            // If we only needed Standard level, return the result
            if original_level == VerificationLevel::Standard {
                return Ok(standard_result);
            }

            // Stage 3: Full verification (if original level is Full and we found serious issues)
            if self.needs_full_verification(&standard_result) {
                let _ = self.tx.send(Event::DebugLog {
                    message: format!(
                        "Progressive verification: Standard found {} issues, escalating to Full",
                        standard_result.discrepancies.len()
                    ),
                    context: HashMap::default(),
                });

                self.config.verification_level = VerificationLevel::Full;
                let full_result = self.verify_with_scope(scope).await?;
                self.config.verification_level = original_level;

                let duration_ms =
                    u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
                let _ = self.tx.send(Event::DebugLog {
                    message: format!(
                        "Progressive verification completed with Full level in {}ms",
                        duration_ms
                    ),
                    context: HashMap::default(),
                });

                return Ok(full_result);
            }

            return Ok(standard_result);
        }

        // Return Quick result if that's all we needed
        Ok(quick_result)
    }

    /// Verify packages in parallel using separate tasks
    ///
    /// This method creates independent verification tasks for each package to avoid
    /// borrowing conflicts while enabling true parallel verification.
    ///
    /// # Errors
    ///
    /// Returns an error if verification fails.
    pub async fn verify_packages_parallel(
        &mut self,
        packages: &[sps2_state::Package],
        scope: &VerificationScope,
    ) -> Result<VerificationResult, Error> {
        // This method always performs parallel verification
        // The decision to use parallel vs sequential should be made by the caller

        let start_time = Instant::now();
        let state_id = self.state_manager.get_active_state().await?;
        let live_path = self.state_manager.live_path().to_path_buf();

        // Emit verification started event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Starting parallel verification for {} packages",
                packages.len()
            ),
            context: HashMap::default(),
        });

        // Pre-fetch all data from database to avoid locking issues
        let _ = self.tx.send(Event::DebugLog {
            message: "Pre-fetching package data and verification cache...".to_string(),
            context: HashMap::default(),
        });

        let mut package_data_list = Vec::new();
        let mut all_file_hashes = HashSet::new();

        // Pre-fetch all package file entries
        let mut db_tx = self.state_manager.begin_transaction().await?;
        for package in packages {
            // Get file entries for this package
            let mut file_entries = queries::get_package_file_entries_by_name(
                &mut db_tx,
                &state_id,
                &package.name,
                &package.version,
            )
            .await?;

            // If not found in current state, try all states
            if file_entries.is_empty() {
                file_entries = queries::get_package_file_entries_all_states(
                    &mut db_tx,
                    &package.name,
                    &package.version,
                )
                .await?;
            }

            // Collect all file hashes for cache lookup
            for entry in &file_entries {
                all_file_hashes.insert(entry.file_hash.clone());
            }

            // Get mtime trackers from database
            let mtime_tracker_list =
                queries::get_package_file_mtimes(&mut db_tx, &package.name, &package.version)
                    .await?;

            let mtime_trackers: HashMap<String, i64> = mtime_tracker_list
                .into_iter()
                .map(|tracker| (tracker.file_path, tracker.last_verified_mtime))
                .collect();

            package_data_list.push(PackageData {
                package: package.clone(),
                file_entries,
                mtime_trackers,
            });
        }

        db_tx.commit().await?;

        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Pre-fetched data for {} packages with {} unique file hashes",
                package_data_list.len(),
                all_file_hashes.len()
            ),
            context: HashMap::default(),
        });

        // Prepare shared data for parallel tasks
        let max_concurrent = self.config.performance.max_concurrent_tasks;
        let verification_level = self.config.verification_level;
        let guard_config = self.config.clone();

        // Create tasks for parallel verification
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut tasks = Vec::new();

        for package_data in package_data_list {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let state_manager = self.state_manager.clone();
            let store = self.store.clone();
            let level = verification_level;
            let config = guard_config.clone();
            let live_path_clone = live_path.clone();
            let state_id_clone = state_id;

            let task = tokio::spawn(async move {
                let _permit = permit; // Hold permit for duration of task

                // Create a minimal verification context for this package
                let result = verify_single_package_with_data(
                    &state_manager,
                    &store,
                    package_data,
                    level,
                    &config,
                    &live_path_clone,
                    &state_id_clone,
                )
                .await;

                result
            });

            tasks.push(task);
        }

        // Collect results from all tasks
        let mut all_discrepancies = Vec::new();
        let mut tracked_files = HashSet::new();
        let mut all_mtime_updates = Vec::new();
        let mut successful_verifications = 0;
        let mut total_cache_hits = 0;
        let mut total_cache_misses = 0;

        for task in tasks {
            match task.await {
                Ok(Ok((package_name, package_version, package_result))) => {
                    successful_verifications += 1;
                    let files_count = package_result.tracked_files.len();
                    all_discrepancies.extend(package_result.discrepancies);
                    tracked_files.extend(package_result.tracked_files);
                    all_mtime_updates.extend(package_result.mtime_updates);
                    total_cache_hits += package_result.cache_hits;
                    total_cache_misses += package_result.cache_misses;

                    let _ = self.tx.send(Event::DebugLog {
                        message: format!(
                            "Successfully verified package {}-{} ({} files)",
                            package_name, package_version, files_count
                        ),
                        context: HashMap::default(),
                    });
                }
                Ok(Err(e)) => {
                    // Even if verification fails, we should have tracked files to avoid false orphans
                    let _ = self.tx.send(Event::DebugLog {
                        message: format!("Package verification error: {}", e),
                        context: HashMap::default(),
                    });
                }
                Err(e) => {
                    let _ = self.tx.send(Event::DebugLog {
                        message: format!("Verification task panicked: {}", e),
                        context: HashMap::default(),
                    });
                }
            }
        }

        // Apply all mtime updates in a single transaction
        if !all_mtime_updates.is_empty() {
            let _ = self.tx.send(Event::DebugLog {
                message: format!("Applying {} mtime updates...", all_mtime_updates.len()),
                context: HashMap::default(),
            });

            // Apply mtime updates to database
            let mut db_tx = self.state_manager.begin_transaction().await?;
            for update in &all_mtime_updates {
                queries::update_file_mtime(&mut db_tx, &update.file_path, update.verified_mtime)
                    .await?;
            }
            db_tx.commit().await?;

            let _ = self.tx.send(Event::DebugLog {
                message: format!(
                    "Applied {} mtime updates to database",
                    all_mtime_updates.len()
                ),
                context: HashMap::default(),
            });
        }

        // Check for orphaned files if not in Quick mode
        if self.level() != VerificationLevel::Quick {
            crate::orphan::detection::find_orphaned_files(
                &live_path,
                &tracked_files,
                &mut all_discrepancies,
            );
        }

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Calculate coverage based on successful verifications
        let total_packages = packages.len();
        let verified_packages = successful_verifications;
        let total_files = tracked_files.len(); // Approximation
        let verified_files = tracked_files.len();

        let orphan_checked_directories = if self.level() != VerificationLevel::Quick {
            vec![live_path.clone()]
        } else {
            vec![]
        };

        let coverage = crate::types::VerificationCoverage::new(
            total_packages,
            verified_packages,
            total_files,
            verified_files,
            orphan_checked_directories,
            matches!(scope, VerificationScope::Full),
        );

        // Calculate cache hit rate
        let cache_hit_rate = if total_cache_hits + total_cache_misses > 0 {
            total_cache_hits as f64 / (total_cache_hits + total_cache_misses) as f64
        } else {
            0.0
        };

        // Emit completion event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Parallel verification completed: {}/{} packages verified in {}ms with {} discrepancies (cache: {:.1}% hit rate, {}/{} hits/total)",
                successful_verifications, total_packages, duration_ms, all_discrepancies.len(),
                cache_hit_rate * 100.0, total_cache_hits, total_cache_hits + total_cache_misses
            ),
            context: HashMap::default(),
        });

        Ok(VerificationResult::with_coverage_and_cache(
            state_id,
            all_discrepancies,
            duration_ms,
            coverage,
            cache_hit_rate,
        ))
    }

    /// Determine if Full verification is needed based on Standard verification results
    fn needs_full_verification(&self, result: &VerificationResult) -> bool {
        // Escalate to Full verification if we find corrupted files or serious issues
        result.discrepancies.iter().any(|d| {
            matches!(
                d,
                crate::types::Discrepancy::CorruptedFile { .. }
                    | crate::types::Discrepancy::MissingFile { .. }
            )
        }) && result.discrepancies.len() > 3 // Only escalate if we have multiple serious issues
    }
}
