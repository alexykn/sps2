//! Types for operations and results

use serde::{Deserialize, Serialize};
use sps2_events::HealthStatus;
use sps2_types::{OpChange, PackageSpec};
use std::collections::HashMap;
use std::path::PathBuf;
// No longer needed - uuid::Uuid imported from sps2_types

/// Operation report for complex operations
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpReport {
    /// Operation type
    pub operation: String,
    /// Whether the operation succeeded
    pub success: bool,
    /// Summary message
    pub summary: String,
    /// Detailed changes
    pub changes: Vec<OpChange>,
    /// Execution time in milliseconds
    pub duration_ms: u64,
}

impl OpReport {
    /// Create success report
    #[must_use]
    pub fn success(
        operation: String,
        summary: String,
        changes: Vec<OpChange>,
        duration_ms: u64,
    ) -> Self {
        Self {
            operation,
            success: true,
            summary,
            changes,
            duration_ms,
        }
    }

    /// Create failure report
    #[must_use]
    pub fn failure(operation: String, summary: String, duration_ms: u64) -> Self {
        Self {
            operation,
            success: false,
            summary,
            changes: Vec::new(),
            duration_ms,
        }
    }
}

// OpChange and ChangeType are now imported from sps2_types

// PackageInfo is now imported from sps2_types

// PackageStatus is now imported from sps2_types

// SearchResult is now imported from sps2_types

// StateInfo is now imported from sps2_types

/// Health check results
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthCheck {
    /// Overall health status
    pub healthy: bool,
    /// Component checks
    pub components: HashMap<String, ComponentHealth>,
    /// Issues found
    pub issues: Vec<HealthIssue>,
}

impl HealthCheck {
    /// Check if system is healthy
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.healthy
    }
}

/// Component health status
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentHealth {
    /// Component name
    pub name: String,
    /// Health status
    pub status: HealthStatus,
    /// Status message
    pub message: String,
    /// Check duration in milliseconds
    pub check_duration_ms: u64,
}

// HealthStatus is now imported from sps2_events

/// Health issue
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthIssue {
    /// Component where issue was found
    pub component: String,
    /// Severity level
    pub severity: IssueSeverity,
    /// Issue description
    pub description: String,
    /// Suggested fix
    pub suggestion: Option<String>,
}

/// Issue severity
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    /// Low severity
    Low,
    /// Medium severity
    Medium,
    /// High severity
    High,
    /// Critical severity
    Critical,
}

/// Install request type
#[derive(Clone, Debug)]
pub enum InstallRequest {
    /// Install from repository
    Remote(PackageSpec),
    /// Install from local file
    LocalFile(PathBuf),
}

// InstallReport is now imported from sps2_types

// PackageChange is now imported from sps2_types

// BuildReport is now imported from sps2_types
