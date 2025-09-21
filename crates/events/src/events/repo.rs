use serde::{Deserialize, Serialize};

/// Repository and index management events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RepoEvent {
    /// Repository synchronization started
    SyncStarted { url: Option<String> },

    /// Repository synchronization completed
    SyncCompleted {
        packages_updated: usize,
        duration_ms: u64,
        bytes_transferred: u64,
    },

    /// Repository synchronization failed
    SyncFailed {
        url: Option<String>,
        retryable: bool,
    },
}
