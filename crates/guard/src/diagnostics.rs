use std::collections::HashMap;
use std::time::Duration;

use sps2_errors::{DiscrepancySeverity, GuardError};

/// Recommended action to take for a discrepancy or guard error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendedAction {
    AutoHeal,
    UserConfirmation,
    ManualIntervention,
    Ignore,
}

/// Simplified contextual information used when emitting guard events.
#[derive(Debug, Clone)]
pub struct DiscrepancyContext {
    pub severity: DiscrepancySeverity,
    pub recommended_action: RecommendedAction,
    pub user_message: String,
    pub technical_details: String,
    pub estimated_fix_time: Option<Duration>,
}

impl DiscrepancyContext {
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
            estimated_fix_time: None,
        }
    }

    pub fn with_estimated_fix_time(mut self, duration: Duration) -> Self {
        self.estimated_fix_time = Some(duration);
        self
    }

    pub fn with_manual_steps(self, _steps: Vec<String>) -> Self {
        self
    }

    pub fn with_prevention_tips(self, _tips: Vec<String>) -> Self {
        self
    }

    pub fn requires_user_action(&self) -> bool {
        matches!(
            self.recommended_action,
            RecommendedAction::UserConfirmation | RecommendedAction::ManualIntervention
        )
    }
}

/// Helper trait providing guard-specific metadata about `GuardError` values.
pub trait GuardErrorExt {
    fn severity(&self) -> DiscrepancySeverity;
    fn is_recoverable(&self) -> bool;
    fn user_context(&self) -> DiscrepancyContext;
}

impl GuardErrorExt for GuardError {
    fn severity(&self) -> DiscrepancySeverity {
        match self {
            GuardError::VerificationFailed { .. } => DiscrepancySeverity::High,
            GuardError::HealingFailed { recoverable, .. } => {
                if *recoverable {
                    DiscrepancySeverity::Medium
                } else {
                    DiscrepancySeverity::High
                }
            }
            GuardError::CacheError { .. } => DiscrepancySeverity::Low,
            GuardError::ConfigurationError { .. } => DiscrepancySeverity::High,
            GuardError::PermissionError { .. } => DiscrepancySeverity::Critical,
            GuardError::ScopeError { .. } | GuardError::TimeoutError { .. } => {
                DiscrepancySeverity::Medium
            }
            GuardError::ResourceExhausted { .. } => DiscrepancySeverity::High,
            GuardError::IntegrityError { severity, .. } => *severity,
            GuardError::StateInconsistency {
                recovery_possible, ..
            } => {
                if *recovery_possible {
                    DiscrepancySeverity::Medium
                } else {
                    DiscrepancySeverity::Critical
                }
            }
            _ => DiscrepancySeverity::Low,
        }
    }

    fn is_recoverable(&self) -> bool {
        matches!(
            self,
            GuardError::HealingFailed {
                recoverable: true,
                ..
            } | GuardError::CacheError { .. }
                | GuardError::ConfigurationError { .. }
                | GuardError::ScopeError { .. }
                | GuardError::TimeoutError { .. }
                | GuardError::ResourceExhausted { .. }
                | GuardError::StateInconsistency {
                    recovery_possible: true,
                    ..
                }
        )
    }

    fn user_context(&self) -> DiscrepancyContext {
        match self {
            GuardError::VerificationFailed {
                operation,
                details,
                discrepancies_count,
                ..
            } => DiscrepancyContext::new(
                DiscrepancySeverity::High,
                RecommendedAction::UserConfirmation,
                format!(
                    "Verification failed during {operation} ({discrepancies_count} discrepancy)"
                ),
                details.clone(),
            ),
            GuardError::HealingFailed {
                discrepancy_type,
                file_path,
                reason,
                recoverable,
            } => {
                let action = if *recoverable {
                    RecommendedAction::UserConfirmation
                } else {
                    RecommendedAction::ManualIntervention
                };
                DiscrepancyContext::new(
                    self.severity(),
                    action,
                    format!("Failed to heal {discrepancy_type} at {file_path}"),
                    reason.clone(),
                )
            }
            GuardError::CacheError { operation, reason } => DiscrepancyContext::new(
                DiscrepancySeverity::Low,
                RecommendedAction::Ignore,
                format!("Cache operation failed: {operation}"),
                reason.clone(),
            ),
            GuardError::ConfigurationError { field, reason, .. } => DiscrepancyContext::new(
                DiscrepancySeverity::High,
                RecommendedAction::ManualIntervention,
                format!("Invalid configuration for '{field}'"),
                reason.clone(),
            ),
            GuardError::PermissionError {
                operation,
                path,
                required_permissions,
            } => DiscrepancyContext::new(
                DiscrepancySeverity::Critical,
                RecommendedAction::ManualIntervention,
                format!("Insufficient permissions for {operation}"),
                format!("Path: {path}. Required permissions: {required_permissions}"),
            ),
            GuardError::ScopeError {
                scope_type,
                reason,
                suggested_scope,
            } => {
                let mut details = reason.clone();
                if let Some(suggestion) = suggested_scope {
                    details.push_str(&format!(". Suggested scope: {suggestion}"));
                }
                DiscrepancyContext::new(
                    DiscrepancySeverity::Medium,
                    RecommendedAction::UserConfirmation,
                    format!("Invalid verification scope: {scope_type}"),
                    details,
                )
            }
            GuardError::TimeoutError {
                operation,
                duration_ms,
                ..
            } => DiscrepancyContext::new(
                DiscrepancySeverity::Medium,
                RecommendedAction::UserConfirmation,
                format!("Guard operation {operation} timed out"),
                format!("Timeout after {duration_ms}ms"),
            ),
            GuardError::ResourceExhausted {
                operation,
                resource_type,
                current_usage,
                limit,
            } => {
                let mut details = format!("Resource exhausted: {resource_type}");
                if let Some(usage) = current_usage {
                    details.push_str(&format!(". Usage: {usage}"));
                }
                if let Some(limit) = limit {
                    details.push_str(&format!(". Limit: {limit}"));
                }
                DiscrepancyContext::new(
                    DiscrepancySeverity::High,
                    RecommendedAction::UserConfirmation,
                    format!("Insufficient resources for {operation}"),
                    details,
                )
            }
            GuardError::IntegrityError {
                component,
                details,
                severity,
            } => DiscrepancyContext::new(
                *severity,
                RecommendedAction::ManualIntervention,
                format!("Integrity check failed for {component}"),
                details.clone(),
            ),
            GuardError::StateInconsistency {
                description,
                current_state,
                expected_state,
                recovery_possible,
            } => {
                let mut details = description.clone();
                if let Some(current) = current_state {
                    details.push_str(&format!(". Current: {current}"));
                }
                if let Some(expected) = expected_state {
                    details.push_str(&format!(". Expected: {expected}"));
                }
                let action = if *recovery_possible {
                    RecommendedAction::UserConfirmation
                } else {
                    RecommendedAction::ManualIntervention
                };
                DiscrepancyContext::new(
                    self.severity(),
                    action,
                    "Guard state inconsistency detected".to_string(),
                    details,
                )
            }
            _ => DiscrepancyContext::new(
                DiscrepancySeverity::Low,
                RecommendedAction::Ignore,
                "Guard error".to_string(),
                String::new(),
            ),
        }
    }
}

/// Aggregated statistics about a collection of guard errors.
#[derive(Debug, Clone)]
pub struct GuardErrorSummary {
    pub total_errors: usize,
    pub overall_severity: DiscrepancySeverity,
    pub recoverable_count: usize,
    pub manual_count: usize,
    pub recommended_actions: Vec<String>,
}

impl GuardErrorSummary {
    pub fn new(errors: Vec<GuardError>) -> Self {
        let total_errors = errors.len();
        let mut by_severity: HashMap<DiscrepancySeverity, usize> = HashMap::new();
        let mut recoverable_count = 0;
        let mut manual_count = 0;

        for error in &errors {
            *by_severity.entry(error.severity()).or_default() += 1;
            if error.is_recoverable() {
                recoverable_count += 1;
            } else {
                manual_count += 1;
            }
        }

        let overall_severity = by_severity
            .keys()
            .max()
            .copied()
            .unwrap_or(DiscrepancySeverity::Low);

        let mut recommended_actions = Vec::new();
        if recoverable_count > 0 {
            recommended_actions.push(format!("Retry auto-heal for {recoverable_count} issue(s)"));
        }
        if manual_count > 0 {
            recommended_actions.push(format!(
                "Manual intervention required for {manual_count} issue(s)"
            ));
        }
        if recommended_actions.is_empty() {
            recommended_actions.push("No action required".to_string());
        }

        Self {
            total_errors,
            overall_severity,
            recoverable_count,
            manual_count,
            recommended_actions,
        }
    }

    pub fn requires_immediate_attention(&self) -> bool {
        matches!(
            self.overall_severity,
            DiscrepancySeverity::Critical | DiscrepancySeverity::High
        )
    }

    pub fn summary_message(&self) -> String {
        if self.total_errors == 0 {
            "No issues found".to_string()
        } else {
            format!(
                "Found {} issue(s); {} recoverable, {} require manual intervention.",
                self.total_errors, self.recoverable_count, self.manual_count
            )
        }
    }
}
