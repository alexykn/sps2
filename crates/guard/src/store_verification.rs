//! Store verification for content-addressed file objects
//!
//! This module provides functionality to verify the integrity of files
//! in the content-addressed store (/opt/pm/store/objects/) by re-hashing
//! them and comparing against their expected hashes.

use sps2_errors::{Error, GuardError};
use sps2_events::{AppEvent, EventEmitter, GeneralEvent, ProgressEvent};
use sps2_hash::Hash;
use sps2_state::{
    file_queries_runtime::{
        get_failed_verification_objects, get_objects_needing_verification, get_verification_stats,
        quarantine_file_object, verify_file_with_tracking,
    },
    StateManager,
};
use sps2_store::FileStore;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// Configuration for store verification
#[derive(Debug, Clone)]
pub struct StoreVerificationConfig {
    /// Maximum age in seconds before re-verification is needed
    pub max_age_seconds: i64,
    /// Maximum verification attempts before quarantining
    pub max_attempts: i32,
    /// Batch size for processing objects
    pub batch_size: i64,
    /// Maximum concurrent verification tasks
    pub max_concurrency: usize,
    /// Whether to quarantine files with repeated failures
    pub enable_quarantine: bool,
}

impl Default for StoreVerificationConfig {
    fn default() -> Self {
        Self {
            max_age_seconds: 30 * 24 * 60 * 60, // 30 days
            max_attempts: 3,
            batch_size: 100,
            max_concurrency: 4,
            enable_quarantine: true,
        }
    }
}

/// Statistics from store verification
#[derive(Debug, Clone)]
pub struct StoreVerificationStats {
    /// Total objects in store
    pub total_objects: i64,
    /// Objects successfully verified
    pub verified_count: i64,
    /// Objects pending verification
    pub pending_count: i64,
    /// Objects with failed verification
    pub failed_count: i64,
    /// Objects quarantined due to repeated failures
    pub quarantined_count: i64,
    /// Objects processed in this run
    pub processed_count: u64,
    /// Objects that passed verification in this run
    pub passed_count: u64,
    /// Objects that failed verification in this run
    pub failed_this_run: u64,
    /// Objects quarantined in this run
    pub quarantined_this_run: u64,
    /// Total time taken for verification
    pub duration: Duration,
    /// Average verification speed (objects/second)
    pub objects_per_second: f64,
}

/// Store verifier for content-addressed file objects
pub struct StoreVerifier {
    state_manager: Arc<StateManager>,
    file_store: Arc<FileStore>,
    config: StoreVerificationConfig,
}

impl StoreVerifier {
    /// Create a new store verifier
    pub fn new(
        state_manager: Arc<StateManager>,
        file_store: Arc<FileStore>,
        config: StoreVerificationConfig,
    ) -> Self {
        Self {
            state_manager,
            file_store,
            config,
        }
    }

    /// Get current verification statistics
    ///
    /// # Errors
    /// Returns an error if database operations fail
    pub async fn get_stats(&self) -> Result<StoreVerificationStats, Error> {
        let mut tx = self.state_manager.begin_transaction().await?;
        let (total, verified, pending, failed, quarantined) =
            get_verification_stats(&mut tx).await?;
        tx.commit().await?;

        Ok(StoreVerificationStats {
            total_objects: total,
            verified_count: verified,
            pending_count: pending,
            failed_count: failed,
            quarantined_count: quarantined,
            processed_count: 0,
            passed_count: 0,
            failed_this_run: 0,
            quarantined_this_run: 0,
            duration: Duration::ZERO,
            objects_per_second: 0.0,
        })
    }

    /// Verify store objects with progress tracking
    ///
    /// # Errors
    /// Returns an error if verification operations fail
    pub async fn verify_with_progress<E>(&self, events: &E) -> Result<StoreVerificationStats, Error>
    where
        E: EventEmitter,
    {
        use sps2_events::events::ProgressPhase;

        let start_time = Instant::now();
        let progress_id = "store-verification";

        // Get initial stats
        let initial_stats = self.get_stats().await?;

        // Start progress tracking
        events.emit(AppEvent::Progress(ProgressEvent::Started {
            id: progress_id.to_string(),
            operation: "Verifying store integrity".to_string(),
            total: Some(initial_stats.pending_count as u64),
            phases: vec![
                ProgressPhase {
                    name: "Scanning".to_string(),
                    weight: 0.1,
                    description: None,
                },
                ProgressPhase {
                    name: "Verifying".to_string(),
                    weight: 0.9,
                    description: None,
                },
            ],
            parent_id: None,
        }));

        // Phase 1: Scanning
        events.emit(AppEvent::Progress(ProgressEvent::Updated {
            id: progress_id.to_string(),
            current: 0,
            total: Some(initial_stats.pending_count as u64),
            phase: Some(0), // Scanning phase index
            speed: None,
            eta: None,
            efficiency: None,
        }));

        // Counters for tracking progress
        let processed_count = Arc::new(AtomicU64::new(0));
        let passed_count = Arc::new(AtomicU64::new(0));
        let failed_count = Arc::new(AtomicU64::new(0));
        let quarantined_count = Arc::new(AtomicU64::new(0));

        // Phase 2: Verifying
        events.emit(AppEvent::Progress(ProgressEvent::Updated {
            id: progress_id.to_string(),
            current: 0,
            total: Some(initial_stats.pending_count as u64),
            phase: Some(1), // Verifying phase index
            speed: None,
            eta: None,
            efficiency: None,
        }));

        // Process objects in batches with concurrency control
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrency));
        let mut _total_processed = 0u64;

        loop {
            // Get next batch of objects needing verification
            let mut tx = self.state_manager.begin_transaction().await?;
            let objects = get_objects_needing_verification(
                &mut tx,
                self.config.max_age_seconds,
                self.config.max_attempts,
                self.config.batch_size,
            )
            .await?;
            tx.commit().await?;

            if objects.is_empty() {
                break;
            }

            // Process batch with concurrency control
            let mut handles = Vec::new();

            for obj in objects {
                let hash = Hash::from_hex(&obj.hash).map_err(|e| {
                    sps2_errors::Error::Guard(GuardError::VerificationFailed {
                        operation: "hash parsing".to_string(),
                        details: format!("invalid hash {}: {e}", obj.hash),
                        discrepancies_count: 0,
                        state_id: "unknown".to_string(),
                        duration_ms: 0,
                    })
                })?;

                let state_manager = Arc::clone(&self.state_manager);
                let file_store = Arc::clone(&self.file_store);
                let semaphore = Arc::clone(&semaphore);
                let processed_count = Arc::clone(&processed_count);
                let passed_count = Arc::clone(&passed_count);
                let failed_count = Arc::clone(&failed_count);
                let quarantined_count = Arc::clone(&quarantined_count);
                let enable_quarantine = self.config.enable_quarantine;
                let max_attempts = self.config.max_attempts;

                let handle = tokio::spawn(async move {
                    let _permit = semaphore.acquire().await.unwrap();

                    let mut tx = state_manager.begin_transaction().await?;
                    let verification_result =
                        verify_file_with_tracking(&mut tx, &file_store, &hash).await;

                    match verification_result {
                        Ok(true) => {
                            // Verification passed
                            tx.commit().await?;
                            passed_count.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(false) => {
                            // Verification failed
                            if enable_quarantine {
                                // Check if we should quarantine this object
                                let failed_objects =
                                    get_failed_verification_objects(&mut tx, 1).await?;
                                if let Some((_, _, attempts)) = failed_objects.first() {
                                    if *attempts >= max_attempts {
                                        quarantine_file_object(
                                            &mut tx,
                                            &hash,
                                            "exceeded maximum verification attempts",
                                        )
                                        .await?;
                                        quarantined_count.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            tx.commit().await?;
                            failed_count.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            // Verification error
                            tx.rollback().await?;
                            return Err(e);
                        }
                    }

                    processed_count.fetch_add(1, Ordering::Relaxed);
                    Ok::<(), Error>(())
                });

                handles.push(handle);
            }

            // Wait for batch to complete
            for handle in handles {
                handle.await.map_err(|e| {
                    sps2_errors::Error::Guard(GuardError::VerificationFailed {
                        operation: "store verification".to_string(),
                        details: format!("task join error: {e}"),
                        discrepancies_count: 0,
                        state_id: "unknown".to_string(),
                        duration_ms: 0,
                    })
                })??;
            }

            _total_processed += self.config.batch_size as u64;

            // Update progress
            let current_processed = processed_count.load(Ordering::Relaxed);
            let elapsed = start_time.elapsed();
            let speed = if elapsed.as_secs() > 0 {
                Some(current_processed as f64 / elapsed.as_secs_f64())
            } else {
                None
            };

            let eta = if let Some(speed_val) = speed {
                if speed_val > 0.0 {
                    let remaining = initial_stats.pending_count as u64 - current_processed;
                    Some(Duration::from_secs_f64(remaining as f64 / speed_val))
                } else {
                    None
                }
            } else {
                None
            };

            events.emit(AppEvent::Progress(ProgressEvent::Updated {
                id: progress_id.to_string(),
                current: current_processed,
                total: Some(initial_stats.pending_count as u64),
                phase: Some(1), // Verifying phase index
                speed,
                eta,
                efficiency: None,
            }));
        }

        let duration = start_time.elapsed();
        let final_processed = processed_count.load(Ordering::Relaxed);
        let objects_per_second = if duration.as_secs() > 0 {
            final_processed as f64 / duration.as_secs_f64()
        } else {
            0.0
        };

        // Complete progress
        events.emit(AppEvent::Progress(ProgressEvent::Completed {
            id: progress_id.to_string(),
            duration,
            final_speed: Some(objects_per_second),
            total_processed: final_processed,
        }));

        // Get final stats
        let final_stats = self.get_stats().await?;

        // Emit completion event
        events.emit(AppEvent::General(GeneralEvent::OperationCompleted {
            operation: format!(
                "Store verification completed: {} objects processed, {} passed, {} failed, {} quarantined in {:.2}s",
                final_processed,
                passed_count.load(Ordering::Relaxed),
                failed_count.load(Ordering::Relaxed),
                quarantined_count.load(Ordering::Relaxed),
                duration.as_secs_f64()
            ),
            success: quarantined_count.load(Ordering::Relaxed) == 0,
        }));

        Ok(StoreVerificationStats {
            total_objects: final_stats.total_objects,
            verified_count: final_stats.verified_count,
            pending_count: final_stats.pending_count,
            failed_count: final_stats.failed_count,
            quarantined_count: final_stats.quarantined_count,
            processed_count: final_processed,
            passed_count: passed_count.load(Ordering::Relaxed),
            failed_this_run: failed_count.load(Ordering::Relaxed),
            quarantined_this_run: quarantined_count.load(Ordering::Relaxed),
            duration,
            objects_per_second,
        })
    }

    /// Get objects with failed verification
    ///
    /// # Errors
    /// Returns an error if database operations fail
    pub async fn get_failed_objects(
        &self,
        limit: i64,
    ) -> Result<Vec<(String, String, i32)>, Error> {
        let mut tx = self.state_manager.begin_transaction().await?;
        let failed_objects = get_failed_verification_objects(&mut tx, limit).await?;
        tx.commit().await?;
        Ok(failed_objects)
    }
}
