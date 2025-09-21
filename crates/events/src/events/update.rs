use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::time::Duration;

/// Update domain events surfaced by ops/update and ops/upgrade
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UpdateEvent {
    /// Update operation started
    Started {
        operation: UpdateOperationType,
        requested: Vec<String>,
        total_targets: usize,
    },

    /// Update operation completed
    Completed {
        operation: UpdateOperationType,
        updated: Vec<UpdateResult>,
        skipped: usize,
        duration: Duration,
        size_difference: i64,
    },

    /// Update operation failed
    Failed {
        operation: UpdateOperationType,
        failure: super::FailureContext,
        updated: Vec<UpdateResult>,
        failed: Vec<String>,
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
