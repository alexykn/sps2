use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::time::Duration;

/// Uninstallation domain events consumed by CLI/logging
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UninstallEvent {
    /// Uninstallation operation started
    Started {
        package: String,
        version: Version,
        force_removal: bool,
        skip_dependency_check: bool,
    },

    /// Uninstallation completed successfully
    Completed {
        package: String,
        version: Version,
        files_removed: usize,
        space_freed: u64,
        duration: Duration,
        dependents_updated: usize,
    },

    /// Uninstallation failed
    Failed {
        package: String,
        version: Version,
        phase: Option<String>,
        error: String,
        cleanup_required: bool,
    },

    /// Batch uninstall started
    BatchStarted {
        packages: Vec<String>,
        operation_id: String,
        dependency_order: bool,
        remove_orphans: bool,
    },

    /// Batch uninstall completed
    BatchCompleted {
        operation_id: String,
        successful_packages: Vec<String>,
        failed_packages: Vec<(String, String)>, // (package, error)
        orphans_removed: Vec<String>,
        total_duration: Duration,
        total_space_freed: u64,
    },
}
