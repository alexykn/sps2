//! Guard-specific error types for state verification and healing operations

use std::time::Duration;
use thiserror::Error;

/// Severity levels for guard-related errors and discrepancies
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DiscrepancySeverity {
    /// Critical - System unusable, immediate action required
    Critical,
    /// High - Major functionality affected, action recommended
    High,
    /// Medium - Minor issues, action optional
    Medium,
    /// Low - Cosmetic issues, informational only
    Low,
}

impl DiscrepancySeverity {
    /// Get human-readable description of severity level
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Critical => "Critical - immediate action required",
            Self::High => "High - action recommended",
            Self::Medium => "Medium - action optional",
            Self::Low => "Low - informational only",
        }
    }

    /// Check if this severity level requires immediate action
    #[must_use]
    pub fn requires_immediate_action(&self) -> bool {
        matches!(self, Self::Critical | Self::High)
    }
}

/// Recommended actions for addressing discrepancies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RecommendedAction {
    /// Automatically heal the discrepancy
    AutoHeal,
    /// Request user confirmation before healing
    UserConfirmation,
    /// Manual intervention required
    ManualIntervention,
    /// Safe to ignore this discrepancy
    Ignore,
}

impl RecommendedAction {
    /// Get human-readable description of the recommended action
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::AutoHeal => "Can be automatically fixed",
            Self::UserConfirmation => "User confirmation recommended before fixing",
            Self::ManualIntervention => "Manual intervention required",
            Self::Ignore => "Safe to ignore",
        }
    }
}

/// Provides detailed, user-friendly context for a given discrepancy.
///
/// This struct is designed to be self-contained, offering all the necessary information
/// to understand and address a specific issue. It includes severity, recommended actions,
/// and both user-facing and technical details.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DiscrepancyContext {
    /// The severity level of this discrepancy, indicating its potential impact.
    pub severity: DiscrepancySeverity,
    /// The recommended course of action to resolve the discrepancy.
    pub recommended_action: RecommendedAction,
    /// A user-friendly message explaining the issue, its impact, and why it matters.
    pub user_message: String,
    /// A technical, detailed explanation of the discrepancy for debugging and analysis.
    pub technical_details: String,
    /// A flag indicating whether an automated healing process is available for this issue.
    pub healing_available: bool,
    /// An optional estimate of the time required to automatically fix the discrepancy.
    pub estimated_fix_time: Option<Duration>,
    /// A list of step-by-step instructions for manual resolution if auto-healing fails or is unavailable.
    pub manual_resolution_steps: Vec<String>,
    /// A list of tips and best practices to help users avoid this issue in the future.
    pub prevention_tips: Vec<String>,
}

impl DiscrepancyContext {
    /// Create a new discrepancy context with basic information
    #[must_use]
    pub fn new(
        severity: DiscrepancySeverity,
        recommended_action: RecommendedAction,
        user_message: String,
        technical_details: String,
    ) -> Self {
        Self {
            severity,
            recommended_action,
            user_message,
            technical_details,
            healing_available: recommended_action != RecommendedAction::ManualIntervention,
            estimated_fix_time: None,
            manual_resolution_steps: Vec::new(),
            prevention_tips: Vec::new(),
        }
    }

    /// Add manual resolution steps
    #[must_use]
    pub fn with_manual_steps(mut self, steps: Vec<String>) -> Self {
        self.manual_resolution_steps = steps;
        self
    }

    /// Add prevention tips
    #[must_use]
    pub fn with_prevention_tips(mut self, tips: Vec<String>) -> Self {
        self.prevention_tips = tips;
        self
    }

    /// Add estimated fix time
    #[must_use]
    pub fn with_estimated_fix_time(mut self, duration: Duration) -> Self {
        self.estimated_fix_time = Some(duration);
        self
    }

    /// Check if user action is required
    #[must_use]
    pub fn requires_user_action(&self) -> bool {
        matches!(
            self.recommended_action,
            RecommendedAction::UserConfirmation | RecommendedAction::ManualIntervention
        )
    }
}

/// Guard-specific error types for comprehensive error reporting
#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GuardError {
    /// Verification operation failed
    #[error("verification failed during {operation}: {details}")]
    VerificationFailed {
        operation: String,
        details: String,
        discrepancies_count: usize,
        state_id: String,
        duration_ms: u64,
    },

    /// Healing operation failed for a specific discrepancy
    #[error("healing failed for {discrepancy_type} at {file_path}: {reason}")]
    HealingFailed {
        discrepancy_type: String,
        file_path: String,
        reason: String,
        recoverable: bool,
        context: Option<Box<DiscrepancyContext>>,
    },

    /// Cache operation failed
    #[error("cache operation failed: {operation} - {reason}")]
    CacheError {
        operation: String,
        reason: String,
        cache_stats: Option<String>,
    },

    /// Invalid guard configuration
    #[error("invalid guard configuration for {field}: {reason}")]
    ConfigurationError {
        field: String,
        reason: String,
        current_value: Option<String>,
        suggested_fix: Option<String>,
    },

    /// Permission denied for guard operation
    #[error("permission denied for {operation} on {path}")]
    PermissionError {
        operation: String,
        path: String,
        required_permissions: String,
        context: Option<String>,
    },

    /// Scope validation failed
    #[error("invalid verification scope: {scope_type} - {reason}")]
    ScopeError {
        scope_type: String,
        reason: String,
        suggested_scope: Option<String>,
    },

    /// Guard operation timed out
    #[error("guard operation timed out: {operation} after {duration_ms}ms")]
    TimeoutError {
        operation: String,
        duration_ms: u64,
        timeout_limit_ms: u64,
    },

    /// Resource exhaustion during guard operation
    #[error("insufficient resources for {operation}: {resource_type}")]
    ResourceExhausted {
        operation: String,
        resource_type: String, // "memory", "disk_space", "file_handles"
        current_usage: Option<String>,
        limit: Option<String>,
    },

    /// Integrity check failed
    #[error("integrity check failed for {component}: {details}")]
    IntegrityError {
        component: String, // "database", "cache", "config"
        details: String,
        severity: DiscrepancySeverity,
    },

    /// Guard state inconsistency
    #[error("guard state inconsistency: {description}")]
    StateInconsistency {
        description: String,
        current_state: Option<String>,
        expected_state: Option<String>,
        recovery_possible: bool,
    },
}

impl GuardError {
    /// Get the severity level of this error
    #[must_use]
    pub fn severity(&self) -> DiscrepancySeverity {
        match self {
            Self::VerificationFailed {
                discrepancies_count,
                ..
            } => {
                if *discrepancies_count > 10 {
                    DiscrepancySeverity::Critical
                } else if *discrepancies_count > 0 {
                    DiscrepancySeverity::High
                } else {
                    DiscrepancySeverity::Medium
                }
            }
            Self::HealingFailed { recoverable, .. } => {
                if *recoverable {
                    DiscrepancySeverity::Medium
                } else {
                    DiscrepancySeverity::High
                }
            }
            Self::CacheError { .. } => DiscrepancySeverity::Low,
            Self::ConfigurationError { .. } | Self::ResourceExhausted { .. } => {
                DiscrepancySeverity::High
            }
            Self::PermissionError { .. } => DiscrepancySeverity::Critical,
            Self::ScopeError { .. } | Self::TimeoutError { .. } => DiscrepancySeverity::Medium,
            Self::IntegrityError { severity, .. } => *severity,
            Self::StateInconsistency {
                recovery_possible, ..
            } => {
                if *recovery_possible {
                    DiscrepancySeverity::Medium
                } else {
                    DiscrepancySeverity::Critical
                }
            }
        }
    }

    /// Check if this error is recoverable through automatic healing
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        match self {
            Self::VerificationFailed { .. }
            | Self::PermissionError { .. }
            | Self::ResourceExhausted { .. }
            | Self::IntegrityError { .. } => false,
            Self::HealingFailed { recoverable, .. } => *recoverable,
            Self::CacheError { .. }
            | Self::ConfigurationError { .. }
            | Self::ScopeError { .. }
            | Self::TimeoutError { .. } => true,
            Self::StateInconsistency {
                recovery_possible, ..
            } => *recovery_possible,
        }
    }

    /// Get user-friendly error context with actionable information
    #[must_use]
    pub fn user_context(&self) -> DiscrepancyContext {
        match self {
            Self::VerificationFailed {
                operation,
                discrepancies_count,
                ..
            } => self.verification_failed_context(operation, *discrepancies_count),
            Self::HealingFailed {
                discrepancy_type,
                file_path,
                reason,
                recoverable,
                ..
            } => self.healing_failed_context(discrepancy_type, file_path, reason, *recoverable),
            Self::PermissionError {
                operation,
                path,
                required_permissions,
                ..
            } => Self::permission_error_context(operation, path, required_permissions),
            Self::ConfigurationError {
                field,
                reason,
                suggested_fix,
                ..
            } => Self::configuration_error_context(field, reason, suggested_fix.as_ref()),
            _ => self.default_context(),
        }
    }

    fn verification_failed_context(
        &self,
        operation: &str,
        discrepancies_count: usize,
    ) -> DiscrepancyContext {
        DiscrepancyContext::new(
            self.severity(),
            RecommendedAction::UserConfirmation,
            format!(
                "System verification failed during {operation} with {discrepancies_count} issue(s) found. Your system may not be functioning correctly."
            ),
            format!("Verification operation: {operation}, discrepancies: {discrepancies_count}"),
        )
        .with_manual_steps(vec![
            "Run 'sps2 verify --heal' to attempt automatic repair".to_string(),
            "Check system logs for specific issues".to_string(),
            "Contact support if issues persist".to_string(),
        ])
        .with_prevention_tips(vec![
            "Run verification regularly to catch issues early".to_string(),
            "Avoid manual modification of package files".to_string(),
        ])
    }

    fn healing_failed_context(
        &self,
        discrepancy_type: &str,
        file_path: &str,
        reason: &str,
        recoverable: bool,
    ) -> DiscrepancyContext {
        let action = if recoverable {
            RecommendedAction::UserConfirmation
        } else {
            RecommendedAction::ManualIntervention
        };

        DiscrepancyContext::new(
            self.severity(),
            action,
            format!(
                "Failed to automatically fix {discrepancy_type} issue for '{file_path}'. {}",
                if recoverable {
                    "Retry may succeed."
                } else {
                    "Manual intervention required."
                }
            ),
            format!("Healing failed: {discrepancy_type} - {reason}"),
        )
        .with_manual_steps(if recoverable {
            vec![
                "Retry the healing operation".to_string(),
                "Check file permissions".to_string(),
                "Ensure sufficient disk space".to_string(),
            ]
        } else {
            vec![
                format!("Manually inspect file: {file_path}"),
                "Check file permissions and ownership".to_string(),
                "Reinstall the affected package if necessary".to_string(),
            ]
        })
    }

    fn permission_error_context(
        operation: &str,
        path: &str,
        required_permissions: &str,
    ) -> DiscrepancyContext {
        DiscrepancyContext::new(
            DiscrepancySeverity::Critical,
            RecommendedAction::ManualIntervention,
            format!(
                "Permission denied for {operation} on '{path}'. Required permissions: {required_permissions}"
            ),
            format!("Operation: {operation}, path: {path}, required: {required_permissions}"),
        )
        .with_manual_steps(vec![
            "Run with appropriate privileges (sudo if needed)".to_string(),
            format!("Check file permissions on: {path}"),
            "Ensure sps2 has necessary access rights".to_string(),
        ])
        .with_prevention_tips(vec![
            "Ensure proper installation permissions".to_string(),
            "Avoid running as root unless necessary".to_string(),
        ])
    }

    fn configuration_error_context(
        field: &str,
        reason: &str,
        suggested_fix: Option<&String>,
    ) -> DiscrepancyContext {
        let mut context = DiscrepancyContext::new(
            DiscrepancySeverity::High,
            RecommendedAction::UserConfirmation,
            format!("Configuration error in '{field}': {reason}"),
            format!("Field: {field}, reason: {reason}"),
        );

        if let Some(fix) = suggested_fix {
            context = context.with_manual_steps(vec![
                format!("Update configuration: {fix}"),
                "Validate configuration with 'sps2 config validate'".to_string(),
            ]);
        }

        context.with_prevention_tips(vec![
            "Use 'sps2 config validate' after making changes".to_string(),
            "Backup configuration before modifications".to_string(),
        ])
    }

    fn default_context(&self) -> DiscrepancyContext {
        DiscrepancyContext::new(
            self.severity(),
            if self.is_recoverable() {
                RecommendedAction::UserConfirmation
            } else {
                RecommendedAction::ManualIntervention
            },
            self.to_string(),
            format!("Error type: {}", std::any::type_name::<Self>()),
        )
    }
}

/// Error aggregation for collecting and categorizing multiple guard errors
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GuardErrorSummary {
    /// Total number of errors
    pub total_errors: usize,
    /// Errors grouped by severity level
    pub by_severity: std::collections::HashMap<DiscrepancySeverity, Vec<GuardError>>,
    /// Errors grouped by error type
    pub by_type: std::collections::HashMap<String, Vec<GuardError>>,
    /// Errors that can be automatically recovered
    pub recoverable_errors: Vec<GuardError>,
    /// Errors requiring manual intervention
    pub manual_intervention_required: Vec<GuardError>,
    /// Overall severity of all errors combined
    pub overall_severity: DiscrepancySeverity,
    /// Suggested next actions
    pub recommended_actions: Vec<String>,
}

impl GuardErrorSummary {
    /// Create a new error summary from a collection of guard errors
    #[must_use]
    pub fn new(errors: Vec<GuardError>) -> Self {
        let total_errors = errors.len();
        let mut by_severity = std::collections::HashMap::new();
        let mut by_type = std::collections::HashMap::new();
        let mut recoverable_errors = Vec::new();
        let mut manual_intervention_required = Vec::new();

        // Categorize errors
        for error in errors {
            let severity = error.severity();
            let error_type = match &error {
                GuardError::VerificationFailed { .. } => "verification",
                GuardError::HealingFailed { .. } => "healing",
                GuardError::CacheError { .. } => "cache",
                GuardError::ConfigurationError { .. } => "configuration",
                GuardError::PermissionError { .. } => "permission",
                GuardError::ScopeError { .. } => "scope",
                GuardError::TimeoutError { .. } => "timeout",
                GuardError::ResourceExhausted { .. } => "resource",
                GuardError::IntegrityError { .. } => "integrity",
                GuardError::StateInconsistency { .. } => "state",
            }
            .to_string();

            by_severity
                .entry(severity)
                .or_insert_with(Vec::new)
                .push(error.clone());
            by_type
                .entry(error_type)
                .or_insert_with(Vec::new)
                .push(error.clone());

            if error.is_recoverable() {
                recoverable_errors.push(error.clone());
            } else {
                manual_intervention_required.push(error);
            }
        }

        // Determine overall severity (highest severity present)
        let overall_severity = by_severity
            .keys()
            .max()
            .copied()
            .unwrap_or(DiscrepancySeverity::Low);

        // Generate recommended actions
        let mut recommended_actions = Vec::new();

        if !recoverable_errors.is_empty() {
            recommended_actions.push(format!(
                "Try automatic healing for {} recoverable issue(s)",
                recoverable_errors.len()
            ));
        }

        if !manual_intervention_required.is_empty() {
            recommended_actions.push(format!(
                "Manual intervention required for {} issue(s)",
                manual_intervention_required.len()
            ));
        }

        if total_errors == 0 {
            recommended_actions.push("No action required".to_string());
        }

        Self {
            total_errors,
            by_severity,
            by_type,
            recoverable_errors,
            manual_intervention_required,
            overall_severity,
            recommended_actions,
        }
    }

    /// Check if any errors require immediate attention
    #[must_use]
    pub fn requires_immediate_attention(&self) -> bool {
        self.overall_severity.requires_immediate_action()
    }

    /// Get a user-friendly summary message
    #[must_use]
    pub fn summary_message(&self) -> String {
        if self.total_errors == 0 {
            "No issues found".to_string()
        } else {
            format!(
                "Found {} issue(s) with {} severity. {} recoverable, {} require manual intervention.",
                self.total_errors,
                self.overall_severity.description(),
                self.recoverable_errors.len(),
                self.manual_intervention_required.len()
            )
        }
    }
}
