//! Main StateVerificationGuard implementation

use crate::cache::VerificationCache;
use crate::types::{
    CacheStats, Discrepancy, HealingContext, VerificationContext, VerificationLevel,
    VerificationResult, VerificationScope,
};
use crate::verification;
use sps2_errors::Error;
use sps2_events::{Event, EventSender};
use sps2_state::{queries, StateManager};
use sps2_store::PackageStore;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// State verification guard for consistency checking
pub struct StateVerificationGuard {
    /// State manager for database operations
    state_manager: StateManager,
    /// Package store for content verification
    store: PackageStore,
    /// Event sender for progress reporting
    tx: EventSender,
    /// Verification level
    level: VerificationLevel,
    /// Verification cache for performance optimization
    cache: VerificationCache,
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
        level: VerificationLevel,
        cache: VerificationCache,
    ) -> Self {
        Self {
            state_manager,
            store,
            tx,
            level,
            cache,
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
        self.level
    }

    /// Verify current state without healing
    ///
    /// # Errors
    ///
    /// Returns an error if state verification fails or database operations fail.
    pub async fn verify_only(&mut self) -> Result<VerificationResult, Error> {
        let start_time = Instant::now();
        let state_id = self.state_manager.get_active_state().await?;
        let live_path = self.state_manager.live_path().to_path_buf();

        // Emit verification started event
        let _ = self.tx.send(Event::DebugLog {
            message: format!("Starting state verification for state {state_id}"),
            context: HashMap::default(),
        });

        // Get all installed packages
        let mut tx = self.state_manager.begin_transaction().await?;
        let packages = queries::get_state_packages(&mut tx, &state_id).await?;
        tx.commit().await?;

        let mut discrepancies = Vec::new();
        let mut tracked_files = HashSet::new();

        // Create verification context
        let mut verification_ctx = VerificationContext {
            state_manager: &self.state_manager,
            store: &self.store,
            cache: &mut self.cache,
            level: self.level,
            state_id: &state_id,
            live_path: &live_path,
        };

        // Verify each package using the verification module
        for package in &packages {
            verification::package::verify_package(
                &mut verification_ctx,
                package,
                &mut discrepancies,
                &mut tracked_files,
            )
            .await?;
        }

        // Check for orphaned files if not in Quick mode
        if self.level != VerificationLevel::Quick {
            crate::orphan::detection::find_orphaned_files(
                &live_path,
                &tracked_files,
                &mut discrepancies,
            );
        }

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Emit verification completed event with cache stats
        let cache_stats = self.cache.stats();
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "State verification completed in {duration_ms}ms with {} discrepancies. Cache: {:.1}% hit rate ({}/{} hits/lookups)",
                discrepancies.len(),
                cache_stats.hit_rate(),
                cache_stats.hits,
                cache_stats.lookups
            ),
            context: HashMap::default(),
        });

        Ok(VerificationResult::new(
            state_id,
            discrepancies,
            duration_ms,
        ))
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
        let start_time = Instant::now();
        let state_id = self.state_manager.get_active_state().await?;
        let live_path = self.state_manager.live_path().to_path_buf();

        // Emit verification started event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Starting scoped state verification for state {state_id} (scope: {:?})",
                scope
            ),
            context: HashMap::default(),
        });

        let mut discrepancies = Vec::new();
        let mut tracked_files = HashSet::new();

        // Get packages based on scope
        let (packages_to_verify, total_packages, total_files) =
            verification::scope::get_packages_for_scope(&self.state_manager, &state_id, scope)
                .await?;

        // Create verification context
        let mut verification_ctx = VerificationContext {
            state_manager: &self.state_manager,
            store: &self.store,
            cache: &mut self.cache,
            level: self.level,
            state_id: &state_id,
            live_path: &live_path,
        };

        // Verify selected packages
        for package in &packages_to_verify {
            verification::package::verify_package(
                &mut verification_ctx,
                package,
                &mut discrepancies,
                &mut tracked_files,
            )
            .await?;
        }

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Calculate coverage
        // Check for orphaned files based on scope
        let orphan_checked_directories = if self.level != VerificationLevel::Quick {
            crate::orphan::detection::find_orphaned_files_scoped(
                scope,
                &live_path,
                &tracked_files,
                &mut discrepancies,
            )
        } else {
            Vec::new()
        };

        let verified_files = tracked_files.len();
        let coverage = crate::types::VerificationCoverage::new(
            total_packages,
            packages_to_verify.len(),
            total_files,
            verified_files,
            orphan_checked_directories,
            matches!(scope, VerificationScope::Full),
        );

        // Emit verification completed event with cache and coverage stats
        let cache_stats = self.cache.stats();
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Scoped verification completed in {duration_ms}ms with {} discrepancies. Coverage: {:.1}% packages ({}/{}), {:.1}% files ({}/{}). Cache: {:.1}% hit rate ({}/{} hits/lookups)",
                discrepancies.len(),
                coverage.package_coverage_percent,
                coverage.verified_packages,
                coverage.total_packages,
                coverage.file_coverage_percent,
                coverage.verified_files,
                coverage.total_files,
                cache_stats.hit_rate(),
                cache_stats.hits,
                cache_stats.lookups
            ),
            context: HashMap::default(),
        });

        Ok(VerificationResult::with_coverage(
            state_id,
            discrepancies,
            duration_ms,
            coverage,
        ))
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
        let start_time = Instant::now();

        // First, run verification to detect discrepancies
        let mut verification_result = self.verify_only().await?;

        // If no discrepancies found, return early
        if verification_result.is_valid {
            return Ok(verification_result);
        }

        // Emit healing start event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Starting healing process for {} discrepancies",
                verification_result.discrepancies.len()
            ),
            context: HashMap::default(),
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
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!(
                                    "Restored missing file: {file_path} from {package_name}-{package_version}"
                                ),
                                context: HashMap::default(),
                            });
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Failed to restore {file_path}: {e}"),
                                context: HashMap::default(),
                            });
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
        verification_result.discrepancies = failed_healings;
        verification_result.is_valid = verification_result.discrepancies.is_empty();

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
        verification_result.duration_ms = duration_ms;

        // Emit healing complete event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Healing completed: {} healed, {} failed in {}ms",
                healed_count,
                verification_result.discrepancies.len(),
                duration_ms
            ),
            context: HashMap::default(),
        });

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

        // Emit healing start event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Starting scoped healing process for {} discrepancies (scope: {:?})",
                verification_result.discrepancies.len(),
                scope
            ),
            context: HashMap::default(),
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
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!(
                                    "Restored missing file: {file_path} from {package_name}-{package_version}"
                                ),
                                context: HashMap::default(),
                            });
                        }
                        Err(e) => {
                            failed_healings.push(discrepancy.clone());
                            let _ = self.tx.send(Event::DebugLog {
                                message: format!("Failed to restore {file_path}: {e}"),
                                context: HashMap::default(),
                            });
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

    /// Get cache statistics
    #[must_use]
    pub fn cache_stats(&self) -> &CacheStats {
        self.cache.stats()
    }

    /// Clear the verification cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Invalidate cache entries for a specific package
    pub fn invalidate_package_cache(&mut self, package_name: &str, package_version: &str) {
        self.cache.invalidate_package(package_name, package_version);
    }

    /// Load cache from persistent storage
    pub async fn load_cache(&mut self) -> Result<(), Error> {
        self.cache.load_from_storage().await
    }

    /// Save cache to persistent storage
    pub async fn save_cache(&self) -> Result<(), Error> {
        self.cache.save_to_storage().await
    }
}
