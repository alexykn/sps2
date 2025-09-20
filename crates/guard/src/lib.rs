//! State verification guard for ensuring database/filesystem consistency

mod core;
mod diagnostics;
mod error_context;
mod healing;
mod orphan;
mod store_verification;
mod types;
mod verification;

// Re-export public types
pub use core::{StateVerificationGuard, StateVerificationGuardBuilder};
pub use diagnostics::{DiscrepancyContext, GuardErrorExt, GuardErrorSummary, RecommendedAction};
pub use error_context::{
    ContextSummaryStats, GuardErrorContext, VerbosityLevel, VerbosityLevelExt,
};
pub use store_verification::{StoreVerificationConfig, StoreVerificationStats, StoreVerifier};
pub use types::{
    derive_post_operation_scope, derive_pre_operation_scope, select_smart_scope, Discrepancy,
    GuardConfig, HealingContext, OperationImpact, OperationResult, OperationType,
    OrphanedFileAction, OrphanedFileCategory, PackageChange, PerformanceConfig, SymlinkPolicy,
    VerificationContext, VerificationCoverage, VerificationLevel, VerificationResult,
    VerificationScope,
};
