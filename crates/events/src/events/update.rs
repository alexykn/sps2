use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::time::Duration;

/// Update domain events surfaced by ops/update and ops/upgrade
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UpdateEvent {
    /// Update operation started
    Started {
        operation_type: UpdateOperationType,
        packages_specified: Vec<String>,
        check_all_packages: bool,
        ignore_constraints: bool,
    },

    /// Update planning phase started
    PlanningStarted {
        packages_to_check: Vec<String>,
        include_dependencies: bool,
    },

    /// Update operation completed
    Completed {
        operation_type: UpdateOperationType,
        packages_updated: Vec<UpdateResult>,
        packages_unchanged: Vec<String>,
        total_duration: Duration,
        space_difference: i64,
    },

    /// Update operation failed
    Failed {
        operation_type: UpdateOperationType,
        failure: super::FailureContext,
        packages_updated: Vec<UpdateResult>,
        packages_failed: Vec<(String, String)>, // (package, error)
    },

    /// Batch update started
    BatchStarted {
        packages: Vec<String>,
        operation_id: String,
        concurrent_limit: usize,
    },

    /// Batch update completed
    BatchCompleted {
        operation_id: String,
        successful_updates: Vec<UpdateResult>,
        failed_updates: Vec<(String, String)>,
        skipped_packages: Vec<String>,
        total_duration: Duration,
        total_size_change: i64,
    },
}

/// Types of update operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateOperationType {
    /// Standard update within version constraints
    Update,
    /// Upgrade ignoring upper bound constraints
    Upgrade,
    /// Downgrade to previous version
    Downgrade,
    /// Reinstall same version (refresh)
    Reinstall,
}

/// Package update types based on semantic versioning
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageUpdateType {
    /// Patch version update (x.y.Z)
    Patch,
    /// Minor version update (x.Y.z)
    Minor,
    /// Major version update (X.y.z)
    Major,
    /// Pre-release version
    PreRelease,
}

/// Update result for completed package updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateResult {
    pub package: String,
    pub from_version: Version,
    pub to_version: Version,
    pub update_type: PackageUpdateType,
    pub duration: Duration,
    pub size_change: i64,
}
