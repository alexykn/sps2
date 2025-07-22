use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::time::Duration;

/// Download-specific events for the event system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DownloadEvent {
    /// Download queued for processing
    Queued {
        url: String,
        package: Option<String>,
        priority: u8,
        queue_position: usize,
        estimated_size: Option<u64>,
    },

    /// Download started with connection info
    Started {
        url: String,
        package: Option<String>,
        total_size: Option<u64>,
        supports_resume: bool,
        connection_time: Duration,
    },

    /// Download progress update with speed/ETA
    Progress {
        url: String,
        bytes_downloaded: u64,
        total_bytes: u64,
        current_speed: f64,
        average_speed: f64,
        eta: Option<Duration>,
    },

    /// Download completed successfully
    Completed {
        url: String,
        package: Option<String>,
        final_size: u64,
        total_time: Duration,
        average_speed: f64,
        hash: String,
    },

    /// Download failed with categorized error
    Failed {
        url: String,
        package: Option<String>,
        error: String,
        error_category: String, // "network", "filesystem", "validation"
        bytes_downloaded: u64,
        recoverable: bool,
    },

    /// Download interrupted but may be resumed
    Interrupted {
        url: String,
        bytes_downloaded: u64,
        reason: String,
        will_resume: bool,
    },

    /// Download resuming from previous attempt
    Resuming {
        url: String,
        resume_offset: u64,
        total_size: Option<u64>,
        attempts_so_far: usize,
    },

    /// Download retrying after failure
    Retrying {
        url: String,
        attempt: usize,
        max_attempts: usize,
        reason: String,
        backoff_delay: Duration,
    },

    /// All retry attempts exhausted
    RetryExhausted {
        url: String,
        total_attempts: usize,
        final_error: String,
        total_time: Duration,
    },

    /// Waiting for resource allocation
    ResourceWaiting {
        url: String,
        resource_type: String,
        estimated_wait: Option<Duration>,
    },

    /// Resource acquired, download can proceed
    ResourceAcquired {
        url: String,
        resource_type: String,
        wait_time: Duration,
    },

    /// Hash verification started
    HashVerificationStarted {
        url: String,
        algorithm: String,
        expected_hash: Option<String>,
    },

    /// Hash verification completed
    HashVerificationCompleted {
        url: String,
        computed_hash: String,
        verification_time: Duration,
        matched: bool,
    },

    /// Hash mismatch detected
    HashMismatch {
        url: String,
        expected: String,
        actual: String,
        action: String, // "deleted_file", "retrying"
    },

    /// Download speed update
    SpeedUpdate {
        url: String,
        current_speed: f64,
        average_speed: f64,
        peak_speed: f64,
        efficiency_rating: f64, // 0.0-1.0
    },

    /// Download has stalled
    Stalled {
        url: String,
        stall_duration: Duration,
        bytes_at_stall: u64,
        suspected_cause: String,
    },

    /// Batch download started
    BatchStarted {
        batch_id: String,
        total_items: usize,
        total_estimated_size: Option<u64>,
        concurrent_limit: usize,
    },

    /// Batch download progress
    BatchProgress {
        batch_id: String,
        completed: usize,
        failed: usize,
        in_progress: usize,
        queued: usize,
        total_bytes_downloaded: u64,
        overall_progress: f64,
    },

    /// Batch download completed
    BatchCompleted {
        batch_id: String,
        successful: usize,
        failed: usize,
        total_time: Duration,
        total_bytes: u64,
        average_speed: f64,
    },

    /// Package-specific download started
    PackageStarted {
        name: String,
        version: Version,
        url: String,
    },

    /// Package download completed
    PackageCompleted { name: String, version: Version },

    /// Package signature downloaded
    SignatureCompleted {
        name: String,
        version: Version,
        verified: bool,
    },
}
