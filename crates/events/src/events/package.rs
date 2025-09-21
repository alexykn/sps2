use serde::{Deserialize, Serialize};

use super::FailureContext;

/// Named package operations surfaced to consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageOperation {
    List,
    Search,
    HealthCheck,
    SelfUpdate,
    Cleanup,
}

/// Outcome payloads for completed operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackageOutcome {
    List {
        total: usize,
    },
    Search {
        query: String,
        total: usize,
    },
    Health {
        healthy: bool,
        issues: Vec<String>,
    },
    SelfUpdate {
        from: String,
        to: String,
        duration_ms: u64,
    },
    Cleanup {
        states_removed: usize,
        packages_removed: usize,
        duration_ms: u64,
    },
}

/// Package-level events consumed by CLI/log handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackageEvent {
    OperationStarted {
        operation: PackageOperation,
    },
    OperationCompleted {
        operation: PackageOperation,
        outcome: PackageOutcome,
    },
    OperationFailed {
        operation: PackageOperation,
        failure: FailureContext,
    },
}

/// Health status indicator for health checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Warning,
    Error,
}
