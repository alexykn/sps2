use serde::{Deserialize, Serialize};

/// Scope covered by a guard operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuardScope {
    System,
    Package {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    Path {
        path: String,
    },
    State {
        id: String,
    },
    Custom {
        description: String,
    },
}

/// Verification depth applied during guard operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "level", content = "details", rename_all = "snake_case")]
pub enum GuardLevel {
    Quick,
    Standard,
    Full,
    Custom(String),
}

/// Summary of verification targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardTargetSummary {
    pub packages: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<usize>,
}

/// Metrics captured at the end of a verification run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardVerificationMetrics {
    pub duration_ms: u64,
    pub cache_hit_rate: f32,
    pub coverage_percent: f32,
}

/// Planned healing workload distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardHealingPlan {
    pub total: usize,
    pub auto_heal: usize,
    pub confirmation_required: usize,
    pub manual_only: usize,
}

/// Severity of a guard discrepancy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Structured description of a guard discrepancy surfaced to consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardDiscrepancy {
    pub kind: String,
    pub severity: GuardSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub message: String,
    pub auto_heal_available: bool,
    pub requires_confirmation: bool,
}

/// Guard events for filesystem integrity verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuardEvent {
    /// Guard verification started.
    VerificationStarted {
        operation_id: String,
        scope: GuardScope,
        level: GuardLevel,
        targets: GuardTargetSummary,
    },

    /// Guard verification completed successfully.
    VerificationCompleted {
        operation_id: String,
        scope: GuardScope,
        discrepancies: usize,
        metrics: GuardVerificationMetrics,
    },

    /// Guard verification failed before completion.
    VerificationFailed {
        operation_id: String,
        scope: GuardScope,
        failure: super::FailureContext,
    },

    /// Healing workflow started.
    HealingStarted {
        operation_id: String,
        plan: GuardHealingPlan,
    },

    /// Healing workflow completed.
    HealingCompleted {
        operation_id: String,
        healed: usize,
        failed: usize,
        duration_ms: u64,
    },

    /// Healing workflow failed prematurely.
    HealingFailed {
        operation_id: String,
        failure: super::FailureContext,
        healed: usize,
    },

    /// Discrepancy discovered during verification or healing.
    DiscrepancyReported {
        operation_id: String,
        discrepancy: GuardDiscrepancy,
    },
}
