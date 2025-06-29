//! Main StateVerificationGuard implementation

use crate::types::{
    Discrepancy, GuardConfig, HealingContext, VerificationContext, VerificationLevel,
    VerificationResult, VerificationScope,
};
use crate::verification;
use sps2_errors::Error;
use sps2_events::{Event, EventSender};
use sps2_state::{queries, StateManager};
use sps2_store::PackageStore;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use uuid;

/// Result of verifying a single package
#[derive(Debug)]
struct SinglePackageResult {
    discrepancies: Vec<Discrepancy>,
    tracked_files: HashSet<std::path::PathBuf>,
}

/// Verify a single package independently (for parallel verification)
async fn verify_single_package_standalone(
    state_manager: &StateManager,
    store: &PackageStore,
    package: &sps2_state::Package,
    level: VerificationLevel,
    guard_config: &GuardConfig,
    live_path: &std::path::Path,
    state_id: &uuid::Uuid,
) -> Result<SinglePackageResult, Error> {
    let mut discrepancies = Vec::new();
    let mut tracked_files: HashSet<std::path::PathBuf> = HashSet::new();

    // Create verification context for this package
    let mut verification_ctx = VerificationContext {
        state_manager,
        store,
        level,
        state_id,
        live_path,
        guard_config,
        tx: None, // No event logging in parallel tasks to avoid conflicts
        scope: &VerificationScope::Full, // Parallel verification always uses full scope
    };

    // Verify the package using the existing verification logic
    let operation_id = "parallel-verify"; // Placeholder for parallel operations
    verification::package::verify_package(
        &mut verification_ctx,
        package,
        &mut discrepancies,
        &mut tracked_files,
        operation_id,
    )
    .await?;

    Ok(SinglePackageResult {
        discrepancies,
        tracked_files,
    })
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
        let start_time = Instant::now();
        let state_id = self.state_manager.get_active_state().await?;
        let live_path = self.state_manager.live_path().to_path_buf();

        // Get all installed packages from current state
        let mut tx = self.state_manager.begin_transaction().await?;
        let packages = queries::get_state_packages(&mut tx, &state_id).await?;
        tx.commit().await?;

        // Emit verification started event
        let operation_id = uuid::Uuid::new_v4().to_string();
        let _ = self.tx.send(Event::GuardVerificationStarted {
            operation_id: operation_id.clone(),
            scope: "system".to_string(),
            level: format!("{:?}", self.config.verification_level),
            packages_count: packages.len(),
            files_count: None,
        });

        let mut discrepancies = Vec::new();
        let mut tracked_files = HashSet::new();

        // Verify each package using the verification module
        for (index, package) in packages.iter().enumerate() {
            // Emit progress update
            let _ = self.tx.send(Event::GuardVerificationProgress {
                operation_id: operation_id.clone(),
                verified_packages: index,
                total_packages: packages.len(),
                verified_files: 0, // TODO: Track file count
                total_files: 0,
                current_package: Some(package.name.clone()),
                cache_hit_rate: None,
            });

            // Create verification context for this package
            let mut verification_ctx = VerificationContext {
                state_manager: &self.state_manager,
                store: &self.store,
                level: self.config.verification_level,
                state_id: &state_id,
                live_path: &live_path,
                guard_config: &self.config,
                tx: Some(&self.tx),
                scope: &VerificationScope::Full,
            };

            verification::package::verify_package(
                &mut verification_ctx,
                package,
                &mut discrepancies,
                &mut tracked_files,
                &operation_id,
            )
            .await?;
        }

        // Check for orphaned files if not in Quick mode
        if self.level() != VerificationLevel::Quick {
            crate::orphan::detection::find_orphaned_files(
                &live_path,
                &tracked_files,
                &mut discrepancies,
            );
        }

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Emit verification completed event
        let by_severity = HashMap::new(); // TODO: Categorize discrepancies by severity
        let _ = self.tx.send(Event::GuardVerificationCompleted {
            operation_id,
            total_discrepancies: discrepancies.len(),
            by_severity,
            duration_ms,
            cache_hit_rate: 0.0,
            coverage_percent: 100.0, // TODO: Calculate actual coverage
            scope_description: format!("System-wide verification of {} packages", packages.len()),
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
            level: self.config.verification_level,
            state_id: &state_id,
            live_path: &live_path,
            guard_config: &self.config,
            tx: Some(&self.tx),
            scope,
        };

        // Verify selected packages
        let operation_id = uuid::Uuid::new_v4().to_string(); // Generate operation ID for scoped verification
        for package in &packages_to_verify {
            verification::package::verify_package(
                &mut verification_ctx,
                package,
                &mut discrepancies,
                &mut tracked_files,
                &operation_id,
            )
            .await?;
        }

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Calculate coverage
        // Check for orphaned files based on scope
        let orphan_checked_directories = if self.level() != VerificationLevel::Quick {
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

        // Emit verification completed event with coverage stats
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Scoped verification completed in {duration_ms}ms with {} discrepancies. Coverage: {:.1}% packages ({}/{}), {:.1}% files ({}/{})",
                discrepancies.len(),
                coverage.package_coverage_percent,
                coverage.verified_packages,
                coverage.total_packages,
                coverage.file_coverage_percent,
                coverage.verified_files,
                coverage.total_files
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
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Processing {} discrepancies for healing",
                discrepancies.len()
            ),
            context: HashMap::default(),
        });

        for discrepancy in &discrepancies {
            let _ = self.tx.send(Event::DebugLog {
                message: format!("Processing discrepancy: {:?}", discrepancy),
                context: HashMap::default(),
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
        if !self.config.performance.parallel_verification || packages.len() <= 1 {
            // Fall back to sequential verification
            return self.verify_with_scope(scope).await;
        }

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

        // Prepare shared data for parallel tasks
        let max_concurrent = self.config.performance.max_concurrent_tasks;
        let verification_level = self.config.verification_level;
        let guard_config = self.config.clone();

        // Create tasks for parallel verification
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut tasks = Vec::new();

        for package in packages.iter().cloned() {
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
                let result = verify_single_package_standalone(
                    &state_manager,
                    &store,
                    &package,
                    level,
                    &config,
                    &live_path_clone,
                    &state_id_clone,
                )
                .await;

                (package.name.clone(), package.version.clone(), result)
            });

            tasks.push(task);
        }

        // Collect results from all tasks
        let mut all_discrepancies = Vec::new();
        let mut tracked_files = HashSet::new();
        let mut successful_verifications = 0;

        for task in tasks {
            match task.await {
                Ok((_package_name, _package_version, Ok(package_result))) => {
                    successful_verifications += 1;
                    all_discrepancies.extend(package_result.discrepancies);
                    tracked_files.extend(package_result.tracked_files);
                }
                Ok((package_name, package_version, Err(e))) => {
                    let _ = self.tx.send(Event::DebugLog {
                        message: format!(
                            "Failed to verify package {}-{}: {}",
                            package_name, package_version, e
                        ),
                        context: HashMap::default(),
                    });
                }
                Err(e) => {
                    let _ = self.tx.send(Event::DebugLog {
                        message: format!("Verification task failed: {}", e),
                        context: HashMap::default(),
                    });
                }
            }
        }

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Calculate coverage based on successful verifications
        let total_packages = packages.len();
        let verified_packages = successful_verifications;
        let total_files = tracked_files.len(); // Approximation
        let verified_files = tracked_files.len();

        let coverage = crate::types::VerificationCoverage::new(
            total_packages,
            verified_packages,
            total_files,
            verified_files,
            vec![], // No orphan checking in parallel mode for now
            matches!(scope, VerificationScope::Full),
        );

        // Emit completion event
        let _ = self.tx.send(Event::DebugLog {
            message: format!(
                "Parallel verification completed: {}/{} packages verified in {}ms with {} discrepancies",
                successful_verifications, total_packages, duration_ms, all_discrepancies.len()
            ),
            context: HashMap::default(),
        });

        Ok(VerificationResult::with_coverage(
            state_id,
            all_discrepancies,
            duration_ms,
            coverage,
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
