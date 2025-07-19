//! Batch operation management and coordination

use sps2_errors::Error;
use sps2_hash::Hash;
use sps2_resolver::PackageId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// State for batch operations
#[derive(Debug)]
pub struct BatchState {
    /// Batch ID
    pub batch_id: String,
    /// Total packages in batch
    pub total_packages: usize,
    /// Completed packages
    pub completed_packages: usize,
    /// Failed packages
    pub failed_packages: Vec<(PackageId, Error)>,
    /// Started time
    pub started_at: Instant,
    /// Rollback capabilities
    pub rollback_info: Option<RollbackInfo>,
}

/// Information needed for rollback
#[derive(Debug)]
pub struct RollbackInfo {
    /// Pre-operation state
    #[allow(dead_code)] // Reserved for rollback to previous state
    pub pre_state: String,
    /// Successfully completed operations that need rollback
    #[allow(dead_code)] // Reserved for tracking operations to rollback
    pub completed_operations: Vec<PackageId>,
    /// Staging directories to clean up
    pub staging_dirs: Vec<PathBuf>,
}

/// Result of batch pipeline execution
#[derive(Debug)]
pub struct BatchResult {
    /// Batch operation ID
    pub batch_id: String,
    /// Successfully processed packages
    pub successful_packages: Vec<PackageId>,
    /// Package hashes for successfully processed packages
    pub package_hashes: HashMap<PackageId, Hash>,
    /// Failed packages with errors
    pub failed_packages: Vec<(PackageId, Error)>,
    /// Total processing time
    pub duration: Duration,
    /// Peak memory usage
    pub peak_memory_usage: u64,
    /// Whether rollback was performed
    pub rollback_performed: bool,
    /// Aggregate statistics
    pub stats: BatchStats,
}

/// Aggregate statistics for batch processing
#[derive(Debug)]
pub struct BatchStats {
    /// Total bytes downloaded
    pub total_downloaded: u64,
    /// Total packages processed
    pub total_packages: usize,
    /// Average download speed (bytes/sec)
    pub avg_download_speed: f64,
    /// Concurrency efficiency (0.0 to 1.0)
    pub concurrency_efficiency: f64,
    /// Time spent in each stage
    pub stage_timings: HashMap<String, Duration>,
}

/// Manager for batch operations
pub struct BatchManager {
    /// Batch operation state
    pub batch_state: RwLock<BatchState>,
}

impl BatchManager {
    /// Create a new batch manager
    pub fn new() -> Self {
        Self {
            batch_state: RwLock::new(BatchState {
                batch_id: "none".to_string(),
                total_packages: 0,
                completed_packages: 0,
                failed_packages: Vec::new(),
                started_at: Instant::now(),
                rollback_info: None,
            }),
        }
    }

    /// Calculate concurrency efficiency based on stage timings
    pub fn calculate_concurrency_efficiency(stage_timings: &HashMap<String, Duration>) -> f64 {
        // Simple efficiency calculation: ratio of parallel to sequential time
        let total_time = stage_timings
            .get("total")
            .map_or(1.0, std::time::Duration::as_secs_f64);
        let sum_stages = stage_timings
            .values()
            .filter_map(|d| {
                if d.as_secs_f64() < total_time {
                    Some(d.as_secs_f64())
                } else {
                    None
                }
            })
            .sum::<f64>();

        if sum_stages > 0.0 {
            (total_time / sum_stages).min(1.0)
        } else {
            0.0
        }
    }
}

impl Default for BatchManager {
    fn default() -> Self {
        Self::new()
    }
}
