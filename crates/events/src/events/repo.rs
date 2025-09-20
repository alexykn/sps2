use serde::{Deserialize, Serialize};

/// Repository and index management events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RepoEvent {
    /// Repository synchronization starting
    SyncStarting,

    /// Repository synchronization started with URL
    SyncStarted { url: String },

    /// Repository synchronization completed
    SyncCompleted {
        packages_updated: usize,
        duration_ms: u64,
        bytes_transferred: u64,
    },
}
