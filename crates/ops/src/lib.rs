#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! High-level operations orchestration for sps2
//!
//! This crate serves as the orchestration layer between the CLI and
//! specialized crates. Small operations are implemented here, while
//! large operations delegate to specialized crates.

mod context;

mod keys;
mod small_ops;

// Import modularized operations
mod health;
mod maintenance;
mod query;
mod repository;
mod security;
mod self_update;
mod types;

// Import command modules
mod build;
mod draft;
mod install;
mod pack;
mod uninstall;
mod update;
mod upgrade;

pub use context::{OpsContextBuilder, OpsCtx};
pub use sps2_guard::{
    Discrepancy, StateVerificationGuard, StateVerificationGuardBuilder, VerificationLevel,
    VerificationResult,
};
// Re-export consolidated types from sps2_types
pub use sps2_types::{
    BuildReport, ChangeType, InstallReport, OpChange, PackageChange, PackageInfo, PackageStatus,
    SearchResult, StateInfo,
};
// Re-export health status from events
pub use sps2_events::events::HealthStatus;
// Re-export ops-specific types from local types module
pub use types::{
    ComponentHealth, HealthCheck, HealthIssue, InstallRequest, IssueSeverity, OpReport, VulnDbStats,
};

// Re-export operation functions
pub use build::build;
pub use draft::draft_recipe;
pub use install::{install, install_with_verification};
pub use pack::{pack_from_directory, pack_from_recipe, pack_from_recipe_no_post};
pub use small_ops::{
    audit, check_health, cleanup, history, list_packages, package_info, reposync, rollback,
    search_packages, self_update, update_vulndb, vulndb_stats,
};
pub use uninstall::{uninstall, uninstall_with_verification};
pub use update::update;
pub use upgrade::{upgrade, upgrade_with_verification};

// Re-export audit types needed by the audit function
pub use sps2_audit::{AuditReport, Severity};

use sps2_errors::Error;

/// Verify the integrity of the current state
///
/// # Errors
///
/// Returns an error if verification fails.
pub async fn verify(ctx: &OpsCtx, heal: bool, level: &str) -> Result<VerificationResult, Error> {
    let verification_level = match level {
        "quick" => VerificationLevel::Quick,
        "full" => VerificationLevel::Full,
        _ => VerificationLevel::Standard,
    };

    let mut guard = StateVerificationGuard::builder()
        .with_state_manager(ctx.state.clone())
        .with_store(ctx.store.clone())
        .with_event_sender(ctx.tx.clone())
        .with_level(verification_level)
        .build()?;

    if heal {
        guard.verify_and_heal(&ctx.config).await
    } else {
        guard.verify_only().await
    }
}
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
    /// Verification result
    VerificationResult(VerificationResult),
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
            OperationResult::VerificationResult(result) => result.is_valid,
        }
    }
}
