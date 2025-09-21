//! Main StateVerificationGuard implementation

use crate::error_context::{GuardErrorContext, VerbosityLevel};
use crate::types::{
    Discrepancy, GuardConfig, HealingContext, OperationType, VerificationLevel, VerificationResult,
    VerificationScope,
};
use crate::verification;
use sps2_errors::Error;
use sps2_events::{
    AppEvent, EventEmitter, EventSender, FailureContext, GuardEvent, GuardHealingPlan, GuardScope,
};
use sps2_hash::Hash;
use sps2_state::{queries, PackageFileEntry, StateManager};
use sps2_store::PackageStore;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use uuid;

/// Check if a file path represents a Python runtime file that gets modified during execution
fn is_python_runtime_file(file_path: &str) -> bool {
    // Python symlinks that get created/modified during runtime
    if file_path == "bin/idle3"
        || file_path == "bin/pydoc3"
        || file_path == "bin/python3"
        || file_path == "bin/python3-config"
    {
        return true;
    }

    // Python pkgconfig files that get modified during runtime
    if file_path == "lib/pkgconfig/python3-embed.pc" || file_path == "lib/pkgconfig/python3.pc" {
        return true;
    }

    // Python man page symlinks that get created/modified during runtime
    if file_path == "share/man/man1/python3.1" {
        return true;
    }

    false
}

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

        tracked_files.insert(std::path::PathBuf::from(file_path));
        let full_path = live_path.join(file_path);

        // Basic existence check
        if !full_path.exists() {
            discrepancies.push(Discrepancy::MissingFile {
                package_name: package.name.clone(),
                package_version: package.version.clone(),
                file_path: file_path.to_string(),
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

            // Check for special file types that require custom handling
            if let Some(special_type) = crate::types::SpecialFileType::from_metadata(&metadata) {
                if special_type.should_skip_verification() {
                    // Log the special file for tracking but skip content verification
                    discrepancies.push(crate::types::Discrepancy::UnsupportedSpecialFile {
                        package_name: package.name.clone(),
                        package_version: package.version.clone(),
                        file_path: file_path.to_string(),
                        file_type: special_type,
                    });
                    continue;
                }
            }

            // Skip Python bytecode files and cache directories from hash verification
            if file_path.ends_with(".pyc") || file_path.contains("__pycache__") {
                continue;
            }

            // Skip Python runtime-generated files that get modified during execution
            if is_python_runtime_file(file_path) {
                continue;
            }

            // Find the file entry for this path
            let file_entry = file_entries
                .iter()
                .find(|entry| entry.relative_path == *file_path);

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
                    if let Some(&last_verified_mtime) = package_data.mtime_trackers.get(file_path) {
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
                            file_path: file_path.to_string(),
                            expected_hash: expected_hash.to_hex(),
                            actual_hash: actual_hash.to_hex(),
                        });
                    }

                    // Always update mtime tracker after verification (success or failure)
                    mtime_updates.push(MTimeUpdate {
                        file_path: file_path.to_string(),
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

impl EventEmitter for StateVerificationGuard {
    fn event_sender(&self) -> Option<&EventSender> {
        Some(&self.tx)
    }
}

impl StateVerificationGuard {
    /// Synchronize DB refcounts to match the active state only (packages and file entries)
    async fn sync_refcounts_active_state(&self) -> Result<(usize, usize), Error> {
        use std::collections::HashMap;

        // Build derived counts and apply updates in a single DB transaction
        let active_state = self.state_manager.get_active_state().await?;
        let mut tx = self.state_manager.begin_transaction().await?;

        // Derive package-level counts by hash for the active state (and capture a size per hash)
        let packages = sps2_state::queries::get_state_packages(&mut tx, &active_state).await?;
        let mut store_counts: HashMap<String, (i64 /*count*/, i64 /*size*/)> = HashMap::new();
        for p in packages {
            store_counts
                .entry(p.hash.clone())
                .and_modify(|e| e.0 += 1)
                .or_insert((1, p.size));
        }

        let mut store_updates = 0usize;
        for (hash, (cnt, size)) in store_counts.iter() {
            // ensure row exists
            sps2_state::queries::get_or_create_store_ref(&mut tx, hash, *size).await?;
            // update only if changed
            let updated = sps2_state::queries::set_store_ref_count(&mut tx, hash, *cnt).await?;
            if updated > 0 {
                store_updates += 1;
            }
        }

        // Derive file-level counts by file_hash for the active state
        let packages_again =
            sps2_state::queries::get_state_packages(&mut tx, &active_state).await?;
        let mut file_counts: HashMap<String, i64> = HashMap::new();
        for p in packages_again {
            let entries =
                sps2_state::file_queries_runtime::get_package_file_entries(&mut tx, p.id).await?;
            for e in entries {
                file_counts
                    .entry(e.file_hash)
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
            }
        }

        let mut file_updates = 0usize;
        for (hash, cnt) in file_counts.iter() {
            let updated =
                sps2_state::file_queries_runtime::set_file_object_ref_count(&mut tx, hash, *cnt)
                    .await?;
            if updated > 0 {
                file_updates += 1;
            }
        }

        tx.commit().await?;
        Ok((store_updates, file_updates))
    }
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
        match self
            .verify_packages_parallel(&packages, &VerificationScope::Full)
            .await
        {
            Ok(verification_result) => {
                let coverage_percent = verification_result
                    .coverage
                    .as_ref()
                    .map(|c| c.package_coverage_percent)
                    .unwrap_or(100.0);
                error_ctx.record_verification_result(&verification_result);
                error_ctx.emit_operation_completed(
                    verification_result.cache_hit_rate,
                    coverage_percent,
                    "system verification",
                );
                error_ctx.emit_error_summary();
                Ok(verification_result)
            }
            Err(error) => {
                let scope = error_ctx
                    .scope()
                    .cloned()
                    .unwrap_or_else(|| GuardScope::Custom {
                        description: "system".to_string(),
                    });
                self.emit(AppEvent::Guard(GuardEvent::VerificationFailed {
                    operation_id: error_ctx.operation_id().to_string(),
                    scope,
                    failure: FailureContext::from_error(&error),
                }));
                Err(error)
            }
        }
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
        self.emit_debug(format!(
            "Starting scoped state verification for state {state_id} (scope: {scope:?})"
        ));

        // Get packages based on scope
        let (packages_to_verify, total_packages, total_files) =
            verification::scope::get_packages_for_scope(&self.state_manager, &state_id, scope)
                .await?;

        // Always use parallel verification (better performance with batched DB writes)
        self.emit_debug(format!(
            "Using parallel verification for {} packages in scope {:?} (max concurrent: {})",
            packages_to_verify.len(),
            scope,
            self.config.performance.max_concurrent_tasks
        ));

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

        // If no discrepancies found, optionally sync refcounts and return early
        if verification_result.is_valid {
            if let Some(guard_cfg) = &config.guard {
                if guard_cfg.store_verification.sync_refcounts {
                    match self.sync_refcounts_active_state().await {
                        Ok((s, f)) => {
                            self.emit_debug(format!(
                                "Guard refcount sync (active-state): store {s}, files {f}"
                            ));
                        }
                        Err(e) => {
                            self.emit_debug(format!("Guard refcount sync failed: {e}"));
                        }
                    }
                }
            }
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

        self.emit(AppEvent::Guard(GuardEvent::HealingStarted {
            operation_id: healing_ctx_events.operation_id().to_string(),
            plan: GuardHealingPlan {
                total: verification_result.discrepancies.len(),
                auto_heal: auto_heal_count,
                confirmation_required,
                manual_only: manual_intervention_count,
            },
        }));

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
        for discrepancy in discrepancies.iter() {
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
                            self.emit_debug(format!("Successfully handled orphaned file: {file_path} (category: {category:?})"));
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            self.emit_debug(format!(
                                "Failed to handle orphaned file {file_path}: {e}"
                            ));
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
                            self.emit_debug(format!("Successfully restored corrupted file: {file_path} for {package_name}-{package_version}"));
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            self.emit_debug(format!(
                                "Failed to restore corrupted file {file_path}: {e}"
                            ));
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
        self.emit(AppEvent::Guard(GuardEvent::HealingCompleted {
            operation_id: healing_ctx_events.operation_id().to_string(),
            healed: healed_count,
            failed: failed_healings.len(),
            duration_ms,
        }));

        // Record healing results in context and emit summary
        for discrepancy in &failed_healings {
            healing_ctx_events.record_discrepancy(discrepancy.clone());
        }
        healing_ctx_events.emit_error_summary();

        // Optional refcount synchronization after healing completes
        if let Some(guard_cfg) = &config.guard {
            if guard_cfg.store_verification.sync_refcounts {
                match self.sync_refcounts_active_state().await {
                    Ok((s, f)) => {
                        self.emit_debug(format!(
                            "Guard refcount sync (active-state): store {s}, files {f}"
                        ));
                    }
                    Err(e) => {
                        self.emit_debug(format!("Guard refcount sync failed: {e}"));
                    }
                }
            }
        }

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

        // If no discrepancies found, optionally sync refcounts and return early
        if verification_result.is_valid {
            if let Some(guard_cfg) = &config.guard {
                if guard_cfg.store_verification.sync_refcounts {
                    match self.sync_refcounts_active_state().await {
                        Ok((s, f)) => {
                            self.emit_debug(format!(
                                "Guard refcount sync (active-state): store {s}, files {f}"
                            ));
                        }
                        Err(e) => {
                            self.emit_debug(format!("Guard refcount sync failed: {e}"));
                        }
                    }
                }
            }
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

        self.emit(AppEvent::Guard(GuardEvent::HealingStarted {
            operation_id: healing_ctx_events.operation_id().to_string(),
            plan: GuardHealingPlan {
                total: verification_result.discrepancies.len(),
                auto_heal: auto_heal_count,
                confirmation_required,
                manual_only: manual_intervention_count,
            },
        }));

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
                            self.emit_debug(format!("Successfully handled orphaned file: {file_path} (category: {category:?})"));
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            self.emit_debug(format!(
                                "Failed to handle orphaned file {file_path}: {e}"
                            ));
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
                            self.emit_debug(format!("Successfully restored corrupted file: {file_path} for {package_name}-{package_version}"));
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            self.emit_debug(format!(
                                "Failed to restore corrupted file {file_path}: {e}"
                            ));
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

        self.emit(AppEvent::Guard(GuardEvent::HealingCompleted {
            operation_id: healing_ctx_events.operation_id().to_string(),
            healed: healed_count,
            failed: verification_result.discrepancies.len(),
            duration_ms,
        }));

        self.emit_debug(format!(
            "Scoped healing completed: {} healed, {} failed in {}ms",
            healed_count,
            verification_result.discrepancies.len(),
            duration_ms
        ));

        // Optional refcount synchronization after healing completes
        if let Some(guard_cfg) = &config.guard {
            if guard_cfg.store_verification.sync_refcounts {
                match self.sync_refcounts_active_state().await {
                    Ok((s, f)) => {
                        self.emit_debug(format!(
                            "Guard refcount sync (active-state): store {s}, files {f}"
                        ));
                    }
                    Err(e) => {
                        self.emit_debug(format!("Guard refcount sync failed: {e}"));
                    }
                }
            }
        }

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
        self.emit_debug("Progressive verification: Starting with Quick level");

        let original_level = self.config.verification_level;
        self.config.verification_level = VerificationLevel::Quick;
        let quick_result = self.verify_with_scope(scope).await?;
        self.config.verification_level = original_level;

        if quick_result.is_valid {
            // No issues found - we're done!
            self.emit_debug(
                "Progressive verification: Quick verification passed, no escalation needed",
            );
            return Ok(quick_result);
        }

        // Stage 2: Standard verification (if issues found and original level >= Standard)
        if original_level >= VerificationLevel::Standard {
            self.emit_debug(format!(
                "Progressive verification: Quick found {} issues, escalating to Standard",
                quick_result.discrepancies.len()
            ));

            self.config.verification_level = VerificationLevel::Standard;
            let standard_result = self.verify_with_scope(scope).await?;
            self.config.verification_level = original_level;

            // If we only needed Standard level, return the result
            if original_level == VerificationLevel::Standard {
                return Ok(standard_result);
            }

            // Stage 3: Full verification (if original level is Full and we found serious issues)
            if self.needs_full_verification(&standard_result) {
                self.emit_debug(format!(
                    "Progressive verification: Standard found {} issues, escalating to Full",
                    standard_result.discrepancies.len()
                ));

                self.config.verification_level = VerificationLevel::Full;
                let full_result = self.verify_with_scope(scope).await?;
                self.config.verification_level = original_level;

                let duration_ms =
                    u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
                self.emit_debug(format!(
                    "Progressive verification completed with Full level in {duration_ms}ms"
                ));

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
        self.emit_debug(format!(
            "Starting parallel verification for {} packages",
            packages.len()
        ));

        // Pre-fetch all data from database to avoid locking issues
        self.emit_debug("Pre-fetching package data and verification cache...");

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

        self.emit_debug(format!(
            "Pre-fetched data for {} packages with {} unique file hashes",
            package_data_list.len(),
            all_file_hashes.len()
        ));

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

                    self.emit_debug(format!(
                        "Successfully verified package {package_name}-{package_version} ({files_count} files)"
                    ));
                }
                Ok(Err(e)) => {
                    // Even if verification fails, we should have tracked files to avoid false orphans
                    self.emit_debug(format!("Package verification error: {e}"));
                }
                Err(e) => {
                    self.emit_debug(format!("Verification task panicked: {e}"));
                }
            }
        }

        // Apply all mtime updates in a single transaction
        if !all_mtime_updates.is_empty() {
            self.emit_debug(format!(
                "Applying {} mtime updates...",
                all_mtime_updates.len()
            ));

            // Apply mtime updates to database
            let mut db_tx = self.state_manager.begin_transaction().await?;
            for update in &all_mtime_updates {
                queries::update_file_mtime(&mut db_tx, &update.file_path, update.verified_mtime)
                    .await?;
            }
            db_tx.commit().await?;

            self.emit_debug(format!(
                "Applied {} mtime updates to database",
                all_mtime_updates.len()
            ));
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
        self.emit_debug(format!(
            "Parallel verification completed: {}/{} packages verified in {}ms with {} discrepancies (cache: {:.1}% hit rate, {}/{} hits/total)",
            successful_verifications, total_packages, duration_ms, all_discrepancies.len(),
            cache_hit_rate * 100.0, total_cache_hits, total_cache_hits + total_cache_misses
        ));

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

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_config::{Config, GuardConfiguration};

    use tempfile::TempDir;
    use tokio::fs as afs;

    async fn mk_env() -> (
        TempDir,
        sps2_state::StateManager,
        sps2_store::PackageStore,
        sps2_events::EventSender,
    ) {
        let td = TempDir::new().unwrap();
        let state = sps2_state::StateManager::new(td.path()).await.unwrap();
        let store_base = td.path().join("store");
        afs::create_dir_all(&store_base).await.unwrap();
        let store = sps2_store::PackageStore::new(store_base);
        let (tx, _rx) = sps2_events::channel();
        (td, state, store, tx)
    }

    #[tokio::test]
    async fn guard_sync_refcounts_active_state_resets_wrong_counts() {
        let (_td, state, store, tx) = mk_env().await;
        // Seed DB: add one package row in active state
        let mut dbtx = state.begin_transaction().await.unwrap();
        let sid = sps2_state::queries::get_active_state(&mut dbtx)
            .await
            .unwrap();
        let pkg_hash = sps2_hash::Hash::from_data(b"pkg-A").to_hex();
        let _pkg_id = sps2_state::queries::add_package(&mut dbtx, &sid, "A", "1.0.0", &pkg_hash, 1)
            .await
            .unwrap();
        // Ensure store_refs row exists and set a wrong count
        sps2_state::queries::get_or_create_store_ref(&mut dbtx, &pkg_hash, 0)
            .await
            .unwrap();
        let _ = sps2_state::queries::set_store_ref_count(&mut dbtx, &pkg_hash, 7)
            .await
            .unwrap();
        dbtx.commit().await.unwrap();

        // Build guard with sync enabled
        let mut guard = StateVerificationGuard::builder()
            .with_state_manager(state.clone())
            .with_store(store.clone())
            .with_event_sender(tx)
            .with_level(crate::types::VerificationLevel::Standard)
            .build()
            .unwrap();
        let mut cfg = Config::default();
        let mut gcfg = GuardConfiguration::default();
        gcfg.store_verification.sync_refcounts = true;
        cfg.guard = Some(gcfg);

        let _ = guard.verify_and_heal(&cfg).await.unwrap();

        let mut check_tx = state.begin_transaction().await.unwrap();
        let rows = sps2_state::queries::get_all_store_refs(&mut check_tx)
            .await
            .unwrap();
        let got = rows.into_iter().find(|r| r.hash == pkg_hash).unwrap();
        assert_eq!(got.ref_count, 1);
    }
}
