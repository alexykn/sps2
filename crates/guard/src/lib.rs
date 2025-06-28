//! State verification guard for ensuring database/filesystem consistency

mod cache;
mod core;
mod error_context;
mod healing;
mod orphan;
mod types;
mod verification;

// Re-export public types
pub use cache::VerificationCache;
pub use core::{StateVerificationGuard, StateVerificationGuardBuilder};
pub use error_context::{
    ContextSummaryStats, GuardErrorContext, VerbosityLevel, VerbosityLevelExt,
};
pub use types::{
    derive_post_operation_scope, derive_pre_operation_scope, select_smart_scope, CacheStats,
    Discrepancy, FileCacheEntry, GuardConfig, HealingContext, OperationImpact, OperationResult,
    OperationType, OrphanedFileAction, OrphanedFileCategory, PackageChange, PerformanceConfig,
    SymlinkPolicy, VerificationContext, VerificationCoverage, VerificationLevel,
    VerificationResult, VerificationScope,
};
