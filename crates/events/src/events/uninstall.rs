use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::path::PathBuf;
use std::time::Duration;

/// Uninstallation domain events - maps to install crate `UninstallOperation` and `sps2 uninstall` command
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
        phase: UninstallPhase,
        error: String,
        partial_removal: bool,
        manual_cleanup_required: bool,
    },

    /// Dependency checking phase started
    DependencyCheckStarted {
        package: String,
        version: Version,
        check_depth: u32,
    },

    /// Dependency check progress
    DependencyCheckProgress {
        package: String,
        packages_checked: usize,
        dependents_found: usize,
        current_package: Option<String>,
    },

    /// Dependency check completed
    DependencyCheckCompleted {
        package: String,
        version: Version,
        direct_dependents: Vec<String>,
        indirect_dependents: Vec<String>,
        safe_to_remove: bool,
        orphaned_packages: Vec<String>,
    },

    /// Dependency check failed
    DependencyCheckFailed {
        package: String,
        version: Version,
        error: String,
        blocking_dependents: Vec<String>,
    },

    /// Blocking dependents detected
    DependentsDetected {
        package: String,
        version: Version,
        blocking_dependents: Vec<(String, Version, String)>, // (name, version, reason)
        suggestions: Vec<String>,
    },

    /// Pre-removal validation started
    ValidationStarted {
        package: String,
        version: Version,
        validation_checks: Vec<String>,
    },

    /// Pre-removal validation completed
    ValidationCompleted {
        package: String,
        version: Version,
        checks_passed: usize,
        warnings: Vec<String>,
        can_proceed: bool,
    },

    /// Pre-removal validation failed
    ValidationFailed {
        package: String,
        version: Version,
        failed_check: String,
        error: String,
        force_override_available: bool,
    },

    /// Removal preparation started
    PreparationStarted {
        package: String,
        version: Version,
        files_to_remove: usize,
        backup_required: bool,
    },

    /// Preparation completed
    PreparationCompleted {
        package: String,
        version: Version,
        backup_created: Option<PathBuf>,
        removal_plan: RemovalPlan,
    },

    /// File removal execution started
    RemovalExecutionStarted {
        package: String,
        version: Version,
        files_to_remove: usize,
        estimated_space_freed: u64,
    },

    /// File removal progress
    RemovalProgress {
        package: String,
        files_removed: usize,
        total_files: usize,
        current_file: Option<PathBuf>,
        space_freed: u64,
    },

    /// File removal execution completed
    RemovalExecutionCompleted {
        package: String,
        version: Version,
        files_removed: usize,
        space_freed: u64,
        directories_cleaned: usize,
    },

    /// Metadata cleanup started
    MetadataCleanupStarted {
        package: String,
        version: Version,
        database_records: usize,
    },

    /// Metadata cleanup completed
    MetadataCleanupCompleted {
        package: String,
        version: Version,
        records_removed: usize,
        indexes_updated: usize,
    },

    /// Orphaned package detected
    OrphanDetected {
        package: String,
        version: Version,
        reason: String,
        auto_removal_eligible: bool,
    },

    /// Orphaned packages cleanup started
    OrphanCleanupStarted {
        orphaned_packages: Vec<String>,
        estimated_space_freed: u64,
    },

    /// Orphaned packages cleanup completed
    OrphanCleanupCompleted {
        removed_packages: Vec<String>,
        space_freed: u64,
        duration: Duration,
    },

    /// Batch uninstall started
    BatchStarted {
        packages: Vec<String>,
        operation_id: String,
        dependency_order: bool,
        remove_orphans: bool,
    },

    /// Batch uninstall progress
    BatchProgress {
        operation_id: String,
        completed_packages: usize,
        failed_packages: usize,
        remaining_packages: usize,
        current_package: Option<String>,
        space_freed_total: u64,
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

    /// Batch uninstall failed
    BatchFailed {
        operation_id: String,
        error: String,
        completed_packages: Vec<String>,
        failed_packages: Vec<(String, String)>,
        cleanup_status: CleanupStatus,
    },

    /// Removal conflict detected
    ConflictDetected {
        package: String,
        version: Version,
        conflict_type: RemovalConflictType,
        affected_files: Vec<PathBuf>,
        resolution_options: Vec<String>,
    },

    /// Conflict resolution applied
    ConflictResolution {
        package: String,
        version: Version,
        resolution_strategy: String,
        files_preserved: usize,
        backup_location: Option<PathBuf>,
    },

    /// Post-removal verification started
    PostRemovalVerification {
        package: String,
        version: Version,
        verification_checks: Vec<String>,
    },

    /// Post-removal verification completed
    PostRemovalVerified {
        package: String,
        version: Version,
        verification_passed: bool,
        residual_files: Vec<PathBuf>,
        system_integrity_ok: bool,
    },
}

/// Uninstallation phases for error reporting and progress tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UninstallPhase {
    /// Checking package dependencies
    DependencyCheck,
    /// Pre-removal validation
    Validation,
    /// Preparing for removal
    Preparation,
    /// Removing files from disk
    RemovalExecution,
    /// Cleaning up metadata
    MetadataCleanup,
    /// Verifying removal completion
    PostRemovalVerification,
}

/// Types of removal conflicts
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalConflictType {
    /// File is shared with another package
    SharedFile,
    /// File has been modified since installation
    ModifiedFile,
    /// Directory is not empty after removal
    NonEmptyDirectory,
    /// Permission denied for removal
    PermissionDenied,
    /// File is currently in use
    FileInUse,
}

/// Removal execution plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovalPlan {
    pub files_to_remove: Vec<PathBuf>,
    pub directories_to_check: Vec<PathBuf>,
    pub shared_files_to_preserve: Vec<PathBuf>,
    pub backup_required: bool,
    pub estimated_duration: Duration,
    pub estimated_space_freed: u64,
}

/// Cleanup status for batch operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupStatus {
    /// All cleanup completed successfully
    Complete,
    /// Partial cleanup completed
    Partial,
    /// Cleanup failed, manual intervention required
    Failed,
    /// Cleanup not attempted
    NotAttempted,
}

impl UninstallEvent {
    /// Create a basic uninstall started event
    pub fn started(package: impl Into<String>, version: Version) -> Self {
        Self::Started {
            package: package.into(),
            version,
            force_removal: false,
            skip_dependency_check: false,
        }
    }

    /// Create a basic uninstall completed event
    pub fn completed(package: impl Into<String>, version: Version, files_removed: usize) -> Self {
        Self::Completed {
            package: package.into(),
            version,
            files_removed,
            space_freed: 0,
            duration: Duration::from_secs(0),
            dependents_updated: 0,
        }
    }

    /// Create a basic uninstall failed event
    pub fn failed(package: impl Into<String>, version: Version, error: impl Into<String>) -> Self {
        Self::Failed {
            package: package.into(),
            version,
            phase: UninstallPhase::RemovalExecution,
            error: error.into(),
            partial_removal: false,
            manual_cleanup_required: false,
        }
    }

    /// Create a dependents detected event
    pub fn dependents_detected(
        package: impl Into<String>,
        version: Version,
        dependents: Vec<String>,
    ) -> Self {
        Self::DependentsDetected {
            package: package.into(),
            version,
            blocking_dependents: dependents
                .into_iter()
                .map(|d| (d, Version::new(0, 0, 0), "dependency".to_string()))
                .collect(),
            suggestions: vec!["Use --force to override".to_string()],
        }
    }
}
