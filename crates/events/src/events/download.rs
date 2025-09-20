use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::time::Duration;

/// Download-specific events surfaced to the CLI and logging pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DownloadEvent {
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

    /// Download resuming from previous attempt
    Resuming {
        url: String,
        resume_offset: u64,
        total_size: Option<u64>,
        attempts_so_far: usize,
    },

    /// Download has stalled
    Stalled {
        url: String,
        stall_duration: Duration,
        bytes_at_stall: u64,
        suspected_cause: String,
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
