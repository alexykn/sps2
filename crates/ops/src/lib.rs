#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! High-level operations orchestration for sps2
//!
//! This crate serves as the orchestration layer between the CLI and
//! specialized crates. Small operations are implemented here, while
//! large operations delegate to specialized crates.

mod context;
mod keys;
mod large_ops;
mod small_ops;
mod types;

pub use context::{OpsContextBuilder, OpsCtx};
pub use types::{
    BuildReport, ChangeType, ComponentHealth, HealthCheck, HealthIssue, HealthStatus,
    InstallReport, InstallRequest, IssueSeverity, OpChange, OpReport, PackageInfo, PackageStatus,
    SearchResult, StateInfo, VulnDbStats,
};

// Re-export operation functions
pub use large_ops::{build, install, uninstall, update, upgrade};
pub use small_ops::{
    audit, check_health, cleanup, history, list_packages, package_info, reposync, rollback,
    search_packages, self_update, update_vulndb, vulndb_stats,
};

// Re-export audit types needed by the audit function
pub use sps2_audit::{AuditReport, Severity};

use sps2_errors::Error;

/// Operation result that can be serialized for CLI output
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type", content = "data")]
pub enum OperationResult {
    /// Package list
    PackageList(Vec<PackageInfo>),
    /// Package information
    PackageInfo(PackageInfo),
    /// Search results
    SearchResults(Vec<SearchResult>),
    /// Installation report
    InstallReport(InstallReport),
    /// Build report
    BuildReport(BuildReport),
    /// State information
    StateInfo(StateInfo),
    /// State history
    StateHistory(Vec<StateInfo>),
    /// Health check results
    HealthCheck(HealthCheck),
    /// Generic success message
    Success(String),
    /// Generic report
    Report(OpReport),
    /// Vulnerability database statistics
    VulnDbStats(VulnDbStats),
    /// Audit report
    AuditReport(sps2_audit::AuditReport),
}

impl OperationResult {
    /// Convert to JSON string
    ///
    /// # Errors
    ///
    /// Returns an error if JSON serialization fails.
    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(self).map_err(|e| {
            sps2_errors::OpsError::SerializationError {
                message: e.to_string(),
            }
            .into()
        })
    }

    /// Check if this is a success result
    #[must_use]
    pub fn is_success(&self) -> bool {
        match self {
            OperationResult::Success(_)
            | OperationResult::PackageList(_)
            | OperationResult::PackageInfo(_)
            | OperationResult::SearchResults(_)
            | OperationResult::InstallReport(_)
            | OperationResult::BuildReport(_)
            | OperationResult::StateInfo(_)
            | OperationResult::StateHistory(_)
            | OperationResult::Report(_)
            | OperationResult::VulnDbStats(_)
            | OperationResult::AuditReport(_) => true,
            OperationResult::HealthCheck(health) => health.is_healthy(),
        }
    }
}
