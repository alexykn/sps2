//! Individual operation tracking for the pipeline

use crate::staging::StagingGuard;
use sps2_resolver::PackageId;
use std::time::Instant;

/// Individual pipeline operation
#[allow(dead_code)] // Reserved for future pipeline operation tracking
pub struct PipelineOperation {
    /// Operation ID
    pub id: String,
    /// Package being processed
    pub package_id: PackageId,
    /// Current stage
    pub stage: PipelineStage,
    /// Started time
    pub started_at: Instant,
    /// Memory usage
    pub memory_usage: u64,
    /// Associated staging directory
    pub staging_guard: Option<StagingGuard>,
    /// Progress tracker ID
    pub progress_id: Option<String>,
}

/// Pipeline processing stages
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)] // Reserved for future pipeline stage tracking
pub enum PipelineStage {
    Queued,
    Downloading,
    StreamingDecompress,
    Validating,
    Staging,
    Installing,
    Completed,
    Failed(String),
}
