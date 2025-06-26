//! State verification guard for ensuring database/filesystem consistency

mod cache;
mod core;
mod healing;
mod orphan;
mod types;
mod verification;

// Re-export public types
pub use cache::VerificationCache;
pub use core::{StateVerificationGuard, StateVerificationGuardBuilder};
pub use types::{
    CacheStats, Discrepancy, FileCacheEntry, HealingContext, OrphanedFileAction,
    OrphanedFileCategory, VerificationContext, VerificationCoverage, VerificationLevel,
    VerificationResult, VerificationScope,
};
