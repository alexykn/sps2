#![warn(mismatched_lifetime_syntaxes)]
#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! High-level operations orchestration for sps2
//!
//! This crate serves as the orchestration layer between the CLI and
//! specialized crates. Small operations are implemented here, while
//! large operations delegate to specialized crates.

mod context;

pub mod keys;
pub mod small_ops;

// Import modularized operations
mod health;
mod maintenance;
mod query;
mod repository;
mod self_update;
mod types;

// Import command modules
mod build;
mod install;
mod pack;
mod uninstall;
mod update;

pub use context::{OpsContextBuilder, OpsCtx};
pub use sps2_guard::{
    Discrepancy, StoreVerificationConfig, StoreVerificationStats, StoreVerifier, VerificationLevel,
    VerificationResult, Verifier,
};
// Re-export consolidated types from sps2_types
pub use sps2_types::{
    BuildReport, ChangeType, InstallReport, OpChange, PackageChange, PackageInfo, PackageStatus,
    SearchResult, StateInfo,
};
// Re-export health status from events
pub use sps2_events::HealthStatus;
// Re-export ops-specific types from local types module
pub use types::{
    ComponentHealth, HealthCheck, HealthIssue, InstallRequest, IssueSeverity, OpReport,
};

// Re-export operation functions
pub use build::build;
pub use install::install;
pub use pack::{pack_from_directory, pack_from_recipe, pack_from_recipe_no_post};
pub use small_ops::{
    check_health, cleanup, history, list_packages, package_info, reposync, rollback,
    search_packages, self_update,
};
pub use uninstall::uninstall;
pub use update::{update, upgrade};

use sps2_errors::Error;
use std::sync::Arc;

/// Verify the integrity of the current state
///
/// # Errors
///
/// Returns an error if verification fails.
#[allow(clippy::cast_possible_truncation)]
pub async fn verify(
    ctx: &OpsCtx,
    heal: bool,
    level: &str,
    scope: &str,
    sync_refcounts: bool,
) -> Result<VerificationResult, Error> {
    let mut verification_level = match level {
        "quick" => VerificationLevel::Quick,
        "full" => VerificationLevel::Full,
        _ => VerificationLevel::Standard,
    };

    if heal {
        verification_level = VerificationLevel::Full;
    }

    match scope {
        "store" => {
            let config = StoreVerificationConfig::default();
            let verifier = StoreVerifier::new(
                Arc::new(ctx.state.clone()),
                Arc::new(ctx.store.file_store().clone()),
                config,
            );

            let stats = verifier.verify_with_progress(&ctx.tx).await?;
            let state_id = ctx.state.get_active_state().await?;

            Ok(VerificationResult::new(
                state_id,
                Vec::new(),
                stats.duration.as_millis() as u64,
            ))
        }
        "all" => {
            let verifier = Verifier::new(ctx.state.clone(), ctx.store.clone(), ctx.tx.clone());
            let result = if heal {
                verifier.verify_and_heal(VerificationLevel::Full).await?
            } else {
                verifier.verify(verification_level).await?
            };

            let config = StoreVerificationConfig::default();
            let store_verifier = StoreVerifier::new(
                Arc::new(ctx.state.clone()),
                Arc::new(ctx.store.file_store().clone()),
                config,
            );
            let _ = store_verifier.verify_with_progress(&ctx.tx).await?;

            if sync_refcounts {
                verifier.sync_refcounts().await?;
            }

            Ok(result)
        }
        _ => {
            let verifier = Verifier::new(ctx.state.clone(), ctx.store.clone(), ctx.tx.clone());
            let result = if heal {
                verifier.verify_and_heal(VerificationLevel::Full).await?
            } else {
                verifier.verify(verification_level).await?
            };

            if sync_refcounts {
                verifier.sync_refcounts().await?;
            }

            Ok(result)
        }
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
            | OperationResult::Report(_) => true,
            OperationResult::HealthCheck(health) => health.is_healthy(),
            OperationResult::VerificationResult(result) => result.is_valid,
        }
    }
}
