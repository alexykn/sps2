use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Parameters for guard discrepancy found events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardDiscrepancyParams {
    pub discrepancy_type: String,
    pub severity: String,
    pub file_path: String,
    pub package: Option<String>,
    pub package_version: Option<String>,
    pub user_message: String,
    pub technical_details: String,
    pub auto_heal_available: bool,
    pub requires_confirmation: bool,
    pub estimated_fix_time_seconds: Option<u64>,
}

/// Guard events for filesystem integrity verification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GuardEvent {
    /// Guard verification started
    VerificationStarted {
        scope: String,
        level: String,
        packages_count: usize,
        files_count: Option<usize>,
    },

    /// Guard discrepancy found
    DiscrepancyFound(GuardDiscrepancyParams),

    /// Guard verification completed
    VerificationCompleted {
        total_discrepancies: usize,
        by_severity: HashMap<String, usize>,
        duration_ms: u64,
        cache_hit_rate: f64,
        coverage_percent: f64,
        scope_description: String,
    },

    /// Guard verification failed
    VerificationFailed {
        retryable: bool,
        packages_verified: usize,
        files_verified: usize,
        duration_ms: u64,
    },

    /// Guard healing started
    HealingStarted {
        discrepancies_count: usize,
        auto_heal_count: usize,
        confirmation_required_count: usize,
        manual_intervention_count: usize,
    },

    /// Guard healing result
    HealingResult {
        discrepancy_type: String,
        file_path: String,
        success: bool,
        healing_action: String,
        error: Option<String>,
        duration_ms: u64,
    },

    /// Guard healing completed
    HealingCompleted {
        healed_count: usize,
        failed_count: usize,
        skipped_count: usize,
        duration_ms: u64,
    },

    /// Guard healing failed
    HealingFailed {
        retryable: bool,
        completed_healing: usize,
        failed_healing: usize,
        duration_ms: u64,
    },
}
