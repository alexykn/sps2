use serde::{Deserialize, Serialize};
/// Download-specific events surfaced to the CLI and logging pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DownloadEvent {
    /// A download began.
    Started {
        url: String,
        package: Option<String>,
        total_bytes: Option<u64>,
    },

    /// The download finished successfully.
    Completed {
        url: String,
        package: Option<String>,
        bytes_downloaded: u64,
    },

    /// The download failed.
    Failed {
        url: String,
        package: Option<String>,
        retryable: bool,
    },
}
