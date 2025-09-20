use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::path::PathBuf;
use std::time::Duration;

/// Package acquisition domain events - higher-level package acquisition from various sources
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AcquisitionEvent {
    /// Package acquisition completed successfully
    Completed {
        package: String,
        version: Version,
        source: AcquisitionSource,
        final_path: PathBuf,
        size: u64,
        duration: Duration,
        verification_passed: bool,
    },

    /// Package acquisition failed
    Failed {
        package: String,
        version: Version,
        source: AcquisitionSource,
        error: String,
        retry_possible: bool,
        partial_download: bool,
    },
}

/// Package acquisition sources
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionSource {
    /// Remote HTTP/HTTPS download
    Remote { url: String, mirror_priority: u8 },
}
