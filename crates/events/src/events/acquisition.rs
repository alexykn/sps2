use serde::{Deserialize, Serialize};
use sps2_types::Version;

/// Package acquisition domain events - higher-level package acquisition from various sources
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AcquisitionEvent {
    /// Package acquisition started
    Started {
        package: String,
        version: Version,
        source: AcquisitionSource,
    },

    /// Package acquisition completed successfully
    Completed {
        package: String,
        version: Version,
        source: AcquisitionSource,
        size: u64,
    },

    /// Package acquisition failed
    Failed {
        package: String,
        version: Version,
        source: AcquisitionSource,
        failure: super::FailureContext,
    },
}

/// Package acquisition sources
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionSource {
    /// Remote HTTP/HTTPS download
    Remote { url: String, mirror_priority: u8 },

    /// Cached package pulled directly from the content-addressed store
    StoreCache { hash: String },
}
