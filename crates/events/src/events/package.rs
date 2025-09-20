use serde::{Deserialize, Serialize};

/// Package operation events surfaced by ops/CLI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PackageEvent {
    /// List operation starting
    ListStarting,

    /// List operation completed
    ListCompleted { count: usize },

    /// Search operation starting
    SearchStarting { query: String },

    /// Search operation completed
    SearchCompleted { query: String, count: usize },

    /// Health check starting
    HealthCheckStarting,

    /// Health check completed
    HealthCheckCompleted { healthy: bool, issues: Vec<String> },

    /// Self-update starting
    SelfUpdateStarting,

    /// Self-update checking version
    SelfUpdateCheckingVersion { current_version: String },

    /// Self-update version available
    SelfUpdateVersionAvailable {
        current_version: String,
        latest_version: String,
    },

    /// Self-update already latest
    SelfUpdateAlreadyLatest { version: String },

    /// Self-update downloading
    SelfUpdateDownloading { version: String, url: String },

    /// Self-update verifying
    SelfUpdateVerifying { version: String },

    /// Self-update installing
    SelfUpdateInstalling { version: String },

    /// Self-update completed
    SelfUpdateCompleted {
        old_version: String,
        new_version: String,
        duration_ms: u64,
    },

    /// Cleanup starting
    CleanupStarting,

    /// Cleanup completed
    CleanupCompleted {
        states_removed: usize,
        packages_removed: usize,
        duration_ms: u64,
    },
}

/// Health status for components used by health reports
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Warning,
    Error,
}
