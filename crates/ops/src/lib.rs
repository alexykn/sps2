#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! High-level operations orchestration for spsv2
//!
//! This crate serves as the orchestration layer between the CLI and
//! specialized crates. Small operations are implemented here, while
//! large operations delegate to specialized crates.

mod context;
mod large_ops;
mod small_ops;
mod types;

pub use context::{OpsContextBuilder, OpsCtx};
pub use types::{
    BuildReport, ChangeType, ComponentHealth, HealthCheck, HealthIssue, HealthStatus, InstallReport, 
    InstallRequest, IssueSeverity, OpChange, OpReport, PackageInfo, PackageStatus, SearchResult,
    StateInfo,
};

// Re-export operation functions
pub use large_ops::{build, install, uninstall, update, upgrade};
pub use small_ops::{
    check_health, cleanup, history, list_packages, package_info, reposync, rollback,
    search_packages,
};

use spsv2_errors::Error;
use spsv2_events::EventSender;

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
}

impl OperationResult {
    /// Convert to JSON string
    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(self).map_err(|e| {
            spsv2_errors::OpsError::SerializationError {
                message: e.to_string(),
            }
            .into()
        })
    }

    /// Check if this is a success result
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
            | OperationResult::Report(_) => true,
            OperationResult::HealthCheck(health) => health.is_healthy(),
        }
    }
}
