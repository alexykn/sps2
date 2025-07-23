use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::time::Duration;

/// Update domain events - maps to ops/update.rs and `sps2 update/upgrade` commands
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

    /// Update operation completed
    Completed {
        operation_type: UpdateOperationType,
        packages_updated: Vec<UpdateResult>,
        packages_unchanged: Vec<String>,
        total_duration: Duration,
        space_difference: i64, // Can be negative if packages got smaller
    },

    /// Update operation failed
    Failed {
        operation_type: UpdateOperationType,
        error: String,
        packages_updated: Vec<UpdateResult>,
        packages_failed: Vec<(String, String)>, // (package, error)
    },

    /// Update planning phase started
    PlanningStarted {
        packages_to_check: Vec<String>,
        include_dependencies: bool,
        constraint_strategy: ConstraintStrategy,
    },

    /// Update planning progress
    PlanningProgress {
        packages_checked: usize,
        total_packages: usize,
        updates_available: usize,
        current_package: Option<String>,
    },

    /// Update plan generated
    PlanGenerated {
        updates_available: Vec<AvailableUpdate>,
        dependency_updates: Vec<AvailableUpdate>,
        conflicts_detected: usize,
        total_estimated_size: u64,
        estimated_duration: Duration,
    },

    /// Update planning failed
    PlanningFailed {
        error: String,
        partial_results: Vec<AvailableUpdate>,
        unresolved_packages: Vec<String>,
    },

    /// Package update started
    PackageUpdateStarted {
        package: String,
        from_version: Version,
        to_version: Version,
        update_type: PackageUpdateType,
    },

    /// Package update progress
    PackageUpdateProgress {
        package: String,
        phase: UpdatePhase,
        progress_percent: f64,
        current_operation: String,
    },

    /// Package update completed
    PackageUpdateCompleted {
        package: String,
        from_version: Version,
        to_version: Version,
        files_changed: usize,
        size_difference: i64,
        duration: Duration,
    },

    /// Package update failed
    PackageUpdateFailed {
        package: String,
        from_version: Version,
        to_version: Version,
        phase: UpdatePhase,
        error: String,
        rollback_attempted: bool,
        rollback_successful: Option<bool>,
    },

    /// Constraint analysis started
    ConstraintAnalysisStarted {
        packages: Vec<String>,
        constraint_type: ConstraintStrategy,
    },

    /// Constraint conflict detected
    ConstraintConflictDetected {
        package: String,
        requested_version: Version,
        available_version: Version,
        constraint_source: String,
        resolution_options: Vec<String>,
    },

    /// Constraint override applied
    ConstraintOverrideApplied {
        package: String,
        original_constraint: String,
        new_constraint: String,
        reason: String,
    },

    /// Batch update started
    BatchStarted {
        packages: Vec<String>,
        operation_id: String,
        update_strategy: BatchUpdateStrategy,
        concurrent_limit: usize,
    },

    /// Batch update progress
    BatchProgress {
        operation_id: String,
        completed_updates: usize,
        failed_updates: usize,
        in_progress_updates: usize,
        remaining_updates: usize,
        current_package: Option<String>,
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

    /// Batch update failed
    BatchFailed {
        operation_id: String,
        error: String,
        completed_updates: Vec<UpdateResult>,
        failed_updates: Vec<(String, String)>,
        rollback_status: RollbackStatus,
    },

    /// Upgrade path analysis (for major version changes)
    UpgradePathAnalysisStarted {
        package: String,
        from_version: Version,
        target_version: Option<Version>, // None means latest
    },

    /// Upgrade path found
    UpgradePathFound {
        package: String,
        from_version: Version,
        to_version: Version,
        intermediate_versions: Vec<Version>,
        breaking_changes: Vec<String>,
        migration_required: bool,
    },

    /// Breaking changes detected
    BreakingChangesDetected {
        package: String,
        from_version: Version,
        to_version: Version,
        breaking_changes: Vec<BreakingChange>,
        migration_strategy: Option<String>,
    },

    /// Dependency cascade update started
    DependencyCascadeStarted {
        root_package: String,
        affected_dependencies: Vec<String>,
        cascade_depth: usize,
    },

    /// Dependency cascade progress
    DependencyCascadeProgress {
        root_package: String,
        dependencies_updated: usize,
        total_dependencies: usize,
        current_dependency: Option<String>,
    },

    /// Dependency cascade completed
    DependencyCascadeCompleted {
        root_package: String,
        dependencies_updated: Vec<UpdateResult>,
        dependencies_skipped: Vec<String>,
        compatibility_preserved: bool,
    },

    /// Rollback initiated due to update failure
    RollbackStarted {
        packages: Vec<String>,
        reason: String,
        rollback_to_snapshot: Option<String>,
    },

    /// Rollback progress
    RollbackProgress {
        packages_restored: usize,
        total_packages: usize,
        current_package: Option<String>,
    },

    /// Rollback completed
    RollbackCompleted {
        packages_restored: Vec<String>,
        restoration_successful: bool,
        duration: Duration,
    },

    /// Rollback failed
    RollbackFailed {
        error: String,
        packages_restored: Vec<String>,
        packages_failed: Vec<String>,
        system_integrity: SystemIntegrityStatus,
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

/// Update execution phases
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    /// Planning and validation
    Planning,
    /// Downloading new version
    Acquisition,
    /// Backing up current version
    Backup,
    /// Installing new version
    Installation,
    /// Verifying update
    Verification,
    /// Cleaning up old version
    Cleanup,
}

/// Constraint handling strategies
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintStrategy {
    /// Respect all version constraints
    Strict,
    /// Ignore upper bound constraints (upgrade mode)
    IgnoreUpperBounds,
    /// Allow pre-release versions
    AllowPreRelease,
    /// Force update regardless of constraints
    Force,
}

/// Batch update strategies
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchUpdateStrategy {
    /// Update packages in dependency order
    DependencyOrder,
    /// Update packages in parallel where possible
    Parallel,
    /// Update packages one by one in specified order
    Sequential,
    /// Update only if all updates can succeed
    AllOrNothing,
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

/// Available update information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableUpdate {
    pub package: String,
    pub current_version: Version,
    pub available_version: Version,
    pub update_type: PackageUpdateType,
    pub size_change: Option<i64>,
    pub breaking_changes: Vec<String>,
    pub urgency: UpdateUrgency,
}

/// Update urgency levels
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateUrgency {
    /// Security or critical bug fix
    Critical,
    /// Important bug fix or feature
    High,
    /// Standard feature update
    Normal,
    /// Minor improvement or enhancement
    Low,
}

/// Breaking change information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakingChange {
    pub change_type: String,
    pub description: String,
    pub migration_hint: Option<String>,
    pub affected_apis: Vec<String>,
}

/// Rollback status for batch operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackStatus {
    /// No rollback needed
    NotRequired,
    /// Rollback completed successfully
    Completed,
    /// Rollback partially completed
    Partial,
    /// Rollback failed
    Failed,
    /// Rollback in progress
    InProgress,
}

/// System integrity status after operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemIntegrityStatus {
    /// System is in a consistent state
    Consistent,
    /// System has minor inconsistencies but is functional
    MinorInconsistencies,
    /// System has major inconsistencies
    MajorInconsistencies,
    /// System integrity cannot be determined
    Unknown,
}

impl UpdateEvent {
    /// Create a basic update started event
    #[must_use]
    pub fn started(packages: Vec<String>, operation_type: UpdateOperationType) -> Self {
        Self::Started {
            operation_type,
            packages_specified: packages,
            check_all_packages: false,
            ignore_constraints: false,
        }
    }

    /// Create an available update from versions
    #[must_use]
    pub fn plan_generated(updates: &[AvailableUpdate]) -> Self {
        let total_size: u64 = updates
            .iter()
            .filter_map(|u| u.size_change)
            .filter(|&s| s > 0)
            .map(|s| s.try_into().unwrap_or(0_u64))
            .sum();

        Self::PlanGenerated {
            updates_available: updates.to_owned(),
            dependency_updates: vec![],
            conflicts_detected: 0,
            total_estimated_size: total_size,
            estimated_duration: Duration::from_secs(updates.len() as u64 * 30), // 30s per package estimate
        }
    }
}
