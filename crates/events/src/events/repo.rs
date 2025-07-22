use serde::{Deserialize, Serialize};

/// Repository and index management events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RepoEvent {
    /// Repository synchronization starting
    SyncStarting,

    /// Repository synchronization started with URL
    SyncStarted { url: String },

    /// Repository synchronization progress
    SyncProgress {
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        current_file: Option<String>,
    },

    /// Repository synchronization completed
    SyncCompleted {
        packages_updated: usize,
        duration_ms: u64,
        bytes_transferred: u64,
    },

    /// Repository synchronization failed
    SyncFailed {
        url: String,
        error: String,
        retry_count: usize,
    },

    /// Index update starting
    IndexUpdateStarting { url: String },

    /// Index update progress
    IndexUpdateProgress {
        processed_entries: usize,
        total_entries: Option<usize>,
    },

    /// Index update completed
    IndexUpdateCompleted {
        packages_added: usize,
        packages_updated: usize,
        packages_removed: usize,
        duration_ms: u64,
    },

    /// Index update failed
    IndexUpdateFailed { url: String, error: String },

    /// Repository validation started
    ValidationStarted { repository_count: usize },

    /// Repository validation progress
    ValidationProgress {
        validated_repos: usize,
        total_repos: usize,
        current_repo: String,
    },

    /// Repository validation completed
    ValidationCompleted {
        valid_repos: usize,
        invalid_repos: usize,
        warnings: Vec<String>,
    },

    /// Repository cache updated
    CacheUpdated {
        cache_type: String, // "metadata", "packages", "signatures"
        entries_updated: usize,
        cache_size_bytes: u64,
    },

    /// Repository cache invalidated
    CacheInvalidated {
        cache_type: String,
        reason: String,
        entries_removed: usize,
    },

    /// Mirror switched
    MirrorSwitched {
        from_url: String,
        to_url: String,
        reason: String,
    },

    /// Mirror health check
    MirrorHealthCheck {
        url: String,
        healthy: bool,
        response_time_ms: u64,
    },
}
