use serde::{Deserialize, Serialize};
use sps2_types::Version;

/// Uninstallation domain events consumed by CLI/logging
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UninstallEvent {
    /// Uninstallation started
    Started { package: String, version: Version },

    /// Uninstallation completed successfully
    Completed {
        package: String,
        version: Version,
        files_removed: usize,
    },

    /// Uninstallation failed
    Failed {
        package: String,
        version: Version,
        retryable: bool,
    },
}
