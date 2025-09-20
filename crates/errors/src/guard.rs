//! Guard-specific error types for state verification and healing operations

use thiserror::Error;

/// Severity levels for guard-related errors and discrepancies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DiscrepancySeverity {
    /// Critical - system unusable, immediate action required
    Critical,
    /// High - major functionality affected, action recommended
    High,
    /// Medium - minor issues, action optional
    Medium,
    /// Low - informational only
    Low,
}

impl DiscrepancySeverity {
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }

    #[must_use]
    pub fn requires_immediate_action(self) -> bool {
        matches!(self, Self::Critical | Self::High)
    }
}

/// Errors emitted by the guard subsystem.
#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum GuardError {
    /// Verification operation failed.
    #[error("verification failed during {operation}: {details}")]
    VerificationFailed {
        operation: String,
        details: String,
        discrepancies_count: usize,
        state_id: String,
        duration_ms: u64,
    },

    /// Healing operation failed for a specific discrepancy.
    #[error("healing failed for {discrepancy_type} at {file_path}: {reason}")]
    HealingFailed {
        discrepancy_type: String,
        file_path: String,
        reason: String,
        recoverable: bool,
    },

    /// Cache operation failed.
    #[error("cache operation failed: {operation} - {reason}")]
    CacheError { operation: String, reason: String },

    /// Invalid guard configuration.
    #[error("invalid guard configuration for {field}: {reason}")]
    ConfigurationError {
        field: String,
        reason: String,
        suggested_fix: Option<String>,
    },

    /// Permission denied for guard operation.
    #[error("permission denied for {operation} on {path}")]
    PermissionError {
        operation: String,
        path: String,
        required_permissions: String,
    },

    /// Scope validation failed.
    #[error("invalid verification scope: {scope_type} - {reason}")]
    ScopeError {
        scope_type: String,
        reason: String,
        suggested_scope: Option<String>,
    },

    /// Guard operation timed out.
    #[error("guard operation timed out: {operation} after {duration_ms}ms")]
    TimeoutError {
        operation: String,
        duration_ms: u64,
        timeout_limit_ms: u64,
    },

    /// Resource exhaustion during guard operation.
    #[error("insufficient resources for {operation}: {resource_type}")]
    ResourceExhausted {
        operation: String,
        resource_type: String,
        current_usage: Option<String>,
        limit: Option<String>,
    },

    /// Integrity check failed.
    #[error("integrity check failed for {component}: {details}")]
    IntegrityError {
        component: String,
        details: String,
        severity: DiscrepancySeverity,
    },

    /// Guard state inconsistency detected.
    #[error("guard state inconsistency: {description}")]
    StateInconsistency {
        description: String,
        current_state: Option<String>,
        expected_state: Option<String>,
        recovery_possible: bool,
    },
}
