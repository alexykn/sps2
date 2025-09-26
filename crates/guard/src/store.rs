use sps2_errors::Error;
use sps2_events::{
    AppEvent, EventEmitter, GuardEvent, GuardLevel, GuardScope, GuardTargetSummary,
    GuardVerificationMetrics,
};
use sps2_hash::Hash;
use sps2_state::{queries, StateManager};
use sps2_store::FileStore;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Configuration for store verification.
#[derive(Debug, Clone)]
pub struct StoreVerificationConfig {
    /// Maximum age in seconds before re-verification is needed.
    pub max_age_seconds: i64,
    /// Maximum verification attempts before we stop retrying.
    pub max_attempts: i32,
    /// Maximum number of objects processed per batch.
    pub batch_size: i64,
}

impl Default for StoreVerificationConfig {
    fn default() -> Self {
        Self {
            max_age_seconds: 30 * 24 * 60 * 60, // 30 days
            max_attempts: 3,
            batch_size: 64,
        }
    }
}

/// Statistics from a store verification run.
#[derive(Debug, Clone)]
pub struct StoreVerificationStats {
    pub total_objects: i64,
    pub verified_count: i64,
    pub pending_count: i64,
    pub failed_count: i64,
    pub quarantined_count: i64,
    pub processed_count: u64,
    pub passed_count: u64,
    pub failed_this_run: u64,
    pub quarantined_this_run: u64,
    pub duration: Duration,
    pub objects_per_second: f64,
}

impl StoreVerificationStats {
    pub fn empty() -> Self {
        Self {
            total_objects: 0,
            verified_count: 0,
            pending_count: 0,
            failed_count: 0,
            quarantined_count: 0,
            processed_count: 0,
            passed_count: 0,
            failed_this_run: 0,
            quarantined_this_run: 0,
            duration: Duration::ZERO,
            objects_per_second: 0.0,
        }
    }
}

/// Minimal verifier for the content-addressed store objects.
pub struct StoreVerifier {
    state_manager: Arc<StateManager>,
    file_store: Arc<FileStore>,
    config: StoreVerificationConfig,
}

impl StoreVerifier {
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

    /// Return summary statistics without performing verification.
    pub async fn get_stats(&self) -> Result<StoreVerificationStats, Error> {
        let mut tx = self.state_manager.begin_transaction().await?;
        let (total, verified, pending, failed, quarantined) =
            queries::get_verification_stats(&mut tx).await?;
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

    /// Run verification over the store and emit guard events describing progress.
    pub async fn verify_with_progress<E>(&self, events: &E) -> Result<StoreVerificationStats, Error>
    where
        E: EventEmitter,
    {
        let initial = self.get_stats().await?;
        let operation_id = Uuid::new_v4().to_string();
        let scope = GuardScope::Custom {
            description: "store objects".to_string(),
        };

        events.emit(AppEvent::Guard(GuardEvent::VerificationStarted {
            operation_id: operation_id.clone(),
            scope: scope.clone(),
            level: GuardLevel::Full,
            targets: GuardTargetSummary {
                packages: 0,
                files: Some(initial.pending_count as usize),
            },
        }));

        let mut processed = 0u64;
        let mut passed = 0u64;
        let mut failed = 0u64;
        let start = Instant::now();

        loop {
            let mut tx = self.state_manager.begin_transaction().await?;
            let objects = queries::get_objects_needing_verification(
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

            for object in objects {
                let hash = match Hash::from_hex(&object.hash) {
                    Ok(h) => h,
                    Err(e) => {
                        failed += 1;
                        processed += 1;
                        events.emit(AppEvent::Guard(GuardEvent::DiscrepancyReported {
                            operation_id: operation_id.clone(),
                            discrepancy: sps2_events::GuardDiscrepancy {
                                kind: "invalid_hash".to_string(),
                                severity: sps2_events::GuardSeverity::High,
                                location: Some(object.hash.clone()),
                                package: None,
                                version: None,
                                message: format!("invalid hash stored in database: {e}"),
                                auto_heal_available: false,
                                requires_confirmation: false,
                            },
                        }));
                        continue;
                    }
                };

                let mut tx = self.state_manager.begin_transaction().await?;
                let verified =
                    queries::verify_file_with_tracking(&mut tx, &self.file_store, &hash).await?;
                tx.commit().await?;

                processed += 1;
                if verified {
                    passed += 1;
                } else {
                    failed += 1;
                    events.emit(AppEvent::Guard(GuardEvent::DiscrepancyReported {
                        operation_id: operation_id.clone(),
                        discrepancy: sps2_events::GuardDiscrepancy {
                            kind: "store_object_failed".to_string(),
                            severity: sps2_events::GuardSeverity::High,
                            location: Some(hash.to_hex()),
                            package: None,
                            version: None,
                            message: "store object failed verification".to_string(),
                            auto_heal_available: false,
                            requires_confirmation: false,
                        },
                    }));
                }
            }
        }

        let duration = start.elapsed();
        let final_stats = self.get_stats().await?;
        let ops_per_sec = if duration.as_secs_f64() > 0.0 {
            processed as f64 / duration.as_secs_f64()
        } else {
            0.0
        };
        let quarantined_run = if final_stats.quarantined_count > initial.quarantined_count {
            (final_stats.quarantined_count - initial.quarantined_count) as u64
        } else {
            0
        };

        events.emit(AppEvent::Guard(GuardEvent::VerificationCompleted {
            operation_id: operation_id.clone(),
            scope,
            discrepancies: failed as usize,
            metrics: GuardVerificationMetrics {
                duration_ms: duration.as_millis() as u64,
                cache_hit_rate: 0.0,
                coverage_percent: 100.0,
            },
        }));

        Ok(StoreVerificationStats {
            total_objects: final_stats.total_objects,
            verified_count: final_stats.verified_count,
            pending_count: final_stats.pending_count,
            failed_count: final_stats.failed_count,
            quarantined_count: final_stats.quarantined_count,
            processed_count: processed,
            passed_count: passed,
            failed_this_run: failed,
            quarantined_this_run: quarantined_run,
            duration,
            objects_per_second: ops_per_sec,
        })
    }
}
