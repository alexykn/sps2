use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Parameters for guard discrepancy found events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardDiscrepancyParams {
    pub operation_id: String,
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
        operation_id: String,
        scope: String,
        level: String,
        packages_count: usize,
        files_count: Option<usize>,
    },

    /// Guard verification progress
    VerificationProgress {
        operation_id: String,
        verified_packages: usize,
        total_packages: usize,
        verified_files: usize,
        total_files: usize,
        current_package: Option<String>,
        cache_hit_rate: Option<f64>,
    },

    /// Guard discrepancy found
    DiscrepancyFound(GuardDiscrepancyParams),

    /// Guard verification completed
    VerificationCompleted {
        operation_id: String,
        total_discrepancies: usize,
        by_severity: HashMap<String, usize>,
        duration_ms: u64,
        cache_hit_rate: f64,
        coverage_percent: f64,
        scope_description: String,
    },

    /// Guard verification failed
    VerificationFailed {
        operation_id: String,
        error: String,
        packages_verified: usize,
        files_verified: usize,
        duration_ms: u64,
    },

    /// Guard healing started
    HealingStarted {
        operation_id: String,
        discrepancies_count: usize,
        auto_heal_count: usize,
        confirmation_required_count: usize,
        manual_intervention_count: usize,
    },

    /// Guard healing progress
    HealingProgress {
        operation_id: String,
        completed: usize,
        total: usize,
        current_operation: String,
        current_file: Option<String>,
    },

    /// Guard healing result
    HealingResult {
        operation_id: String,
        discrepancy_type: String,
        file_path: String,
        success: bool,
        healing_action: String,
        error: Option<String>,
        duration_ms: u64,
    },

    /// Guard healing completed
    HealingCompleted {
        operation_id: String,
        healed_count: usize,
        failed_count: usize,
        skipped_count: usize,
        duration_ms: u64,
    },

    /// Guard healing failed
    HealingFailed {
        operation_id: String,
        error: String,
        completed_healing: usize,
        failed_healing: usize,
        duration_ms: u64,
    },

    /// Guard error summary
    ErrorSummary {
        operation_id: String,
        total_errors: usize,
        recoverable_errors: usize,
        manual_intervention_required: usize,
        overall_severity: String,
        user_friendly_summary: String,
        recommended_actions: Vec<String>,
    },
}

impl GuardEvent {
    /// Create a guard discrepancy found event
    #[must_use]
    pub fn discrepancy_found(params: GuardDiscrepancyParams) -> Self {
        Self::DiscrepancyFound(params)
    }

    /// Create a guard verification started event
    pub fn verification_started(
        operation_id: impl Into<String>,
        scope: impl Into<String>,
        level: impl Into<String>,
        packages_count: usize,
        files_count: Option<usize>,
    ) -> Self {
        Self::VerificationStarted {
            operation_id: operation_id.into(),
            scope: scope.into(),
            level: level.into(),
            packages_count,
            files_count,
        }
    }

    /// Create a guard verification completed event
    pub fn verification_completed(
        operation_id: impl Into<String>,
        total_discrepancies: usize,
        by_severity: HashMap<String, usize>,
        duration_ms: u64,
        cache_hit_rate: f64,
        coverage_percent: f64,
        scope_description: impl Into<String>,
    ) -> Self {
        Self::VerificationCompleted {
            operation_id: operation_id.into(),
            total_discrepancies,
            by_severity,
            duration_ms,
            cache_hit_rate,
            coverage_percent,
            scope_description: scope_description.into(),
        }
    }

    /// Create a guard healing result event
    pub fn healing_result(
        operation_id: impl Into<String>,
        discrepancy_type: impl Into<String>,
        file_path: impl Into<String>,
        success: bool,
        healing_action: impl Into<String>,
        error: Option<String>,
        duration_ms: u64,
    ) -> Self {
        Self::HealingResult {
            operation_id: operation_id.into(),
            discrepancy_type: discrepancy_type.into(),
            file_path: file_path.into(),
            success,
            healing_action: healing_action.into(),
            error,
            duration_ms,
        }
    }
}
