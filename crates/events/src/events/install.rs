use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::path::PathBuf;
use std::time::Duration;

/// Installation domain events - maps to install crate and `sps2 install` command
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InstallEvent {
    /// Installation operation started for a package
    Started {
        package: String,
        version: Version,
        install_path: PathBuf,
        force_reinstall: bool,
    },

    /// Installation completed successfully
    Completed {
        package: String,
        version: Version,
        installed_files: usize,
        install_path: PathBuf,
        duration: Duration,
        disk_usage: u64,
    },

    /// Installation failed
    Failed {
        package: String,
        version: Version,
        phase: InstallPhase,
        error: String,
        cleanup_required: bool,
    },

    /// Package staging phase started
    StagingStarted {
        package: String,
        version: Version,
        source_path: PathBuf,
        staging_path: PathBuf,
    },

    /// Package staging progress update
    StagingProgress {
        package: String,
        files_extracted: usize,
        total_files: Option<usize>,
        current_file: Option<String>,
        bytes_extracted: u64,
    },

    /// Package staging completed
    StagingCompleted {
        package: String,
        version: Version,
        files_staged: usize,
        staging_size: u64,
        staging_path: PathBuf,
    },

    /// Package staging failed
    StagingFailed {
        package: String,
        version: Version,
        error: String,
        files_partially_extracted: usize,
    },

    /// Installation preparation phase
    PreparationStarted {
        package: String,
        version: Version,
        target_path: PathBuf,
        backup_existing: bool,
    },

    /// Preparation phase completed
    PreparationCompleted {
        package: String,
        version: Version,
        backup_created: Option<PathBuf>,
        space_required: u64,
        space_available: u64,
    },

    /// File installation phase executing
    FileInstallationStarted {
        package: String,
        version: Version,
        files_to_install: usize,
        estimated_size: u64,
    },

    /// File installation progress
    FileInstallationProgress {
        package: String,
        files_installed: usize,
        total_files: usize,
        current_file: Option<String>,
        bytes_written: u64,
    },

    /// File installation completed
    FileInstallationCompleted {
        package: String,
        version: Version,
        files_installed: usize,
        bytes_written: u64,
    },

    /// Metadata registration started
    MetadataRegistrationStarted {
        package: String,
        version: Version,
        dependencies: usize,
    },

    /// Metadata registration completed
    MetadataRegistrationCompleted {
        package: String,
        version: Version,
        database_records: usize,
    },

    /// Post-installation validation started
    ValidationStarted {
        package: String,
        version: Version,
        validation_checks: Vec<String>,
    },

    /// Post-installation validation progress
    ValidationProgress {
        package: String,
        checks_completed: usize,
        total_checks: usize,
        current_check: String,
    },

    /// Post-installation validation completed
    ValidationCompleted {
        package: String,
        version: Version,
        checks_passed: usize,
        warnings: usize,
        issues_found: usize,
    },

    /// Post-installation validation failed
    ValidationFailed {
        package: String,
        version: Version,
        failed_check: String,
        error: String,
        can_continue: bool,
    },

    /// Batch installation started
    BatchStarted {
        packages: Vec<String>,
        operation_id: String,
        concurrent_limit: usize,
        estimated_duration: Option<Duration>,
    },

    /// Batch installation progress
    BatchProgress {
        operation_id: String,
        completed_packages: usize,
        failed_packages: usize,
        in_progress_packages: usize,
        remaining_packages: usize,
        current_package: Option<String>,
    },

    /// Batch installation completed
    BatchCompleted {
        operation_id: String,
        successful_packages: Vec<String>,
        failed_packages: Vec<(String, String)>, // (package, error)
        total_duration: Duration,
        total_disk_usage: u64,
    },

    /// Batch installation failed
    BatchFailed {
        operation_id: String,
        error: String,
        completed_packages: Vec<String>,
        failed_packages: Vec<(String, String)>,
        rollback_initiated: bool,
    },

    /// Rollback started due to installation failure
    RollbackStarted {
        package: String,
        version: Version,
        reason: String,
        backup_path: Option<PathBuf>,
    },

    /// Rollback completed
    RollbackCompleted {
        package: String,
        version: Version,
        files_restored: usize,
        cleanup_successful: bool,
    },

    /// Rollback failed
    RollbackFailed {
        package: String,
        version: Version,
        error: String,
        manual_cleanup_required: bool,
    },

    /// Package conflict detected during installation
    ConflictDetected {
        package: String,
        version: Version,
        conflict_type: InstallConflictType,
        conflicting_package: Option<String>,
        conflicting_files: Vec<PathBuf>,
    },

    /// Conflict resolution attempted
    ConflictResolution {
        package: String,
        version: Version,
        resolution_strategy: String,
        backup_created: bool,
    },

    /// Permission adjustment required
    PermissionAdjustment {
        package: String,
        files_affected: usize,
        permission_type: String, // "executable", "readable", "ownership"
        requires_root: bool,
    },
}

/// Installation phases for error reporting and progress tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    /// Package acquisition and validation
    Acquisition,
    /// Extracting and staging files
    Staging,
    /// Preparing for installation
    Preparation,
    /// Installing files to final location
    FileInstallation,
    /// Registering package metadata
    MetadataRegistration,
    /// Post-installation validation
    Validation,
    /// Rollback due to failure
    Rollback,
}

/// Types of installation conflicts
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallConflictType {
    /// File already exists and owned by another package
    FileConflict,
    /// Package version already installed
    VersionConflict,
    /// Dependency version conflicts with existing installation
    DependencyConflict,
    /// Insufficient disk space
    SpaceConflict,
    /// Permission denied for required operation
    PermissionConflict,
    /// System compatibility issues
    SystemConflict,
}

impl InstallEvent {
    /// Create a basic installation started event
    pub fn started(package: impl Into<String>, version: Version) -> Self {
        Self::Started {
            package: package.into(),
            version,
            install_path: PathBuf::from("/opt/pm/live"), // Default install path
            force_reinstall: false,
        }
    }

    /// Create a basic installation completed event
    pub fn completed(package: impl Into<String>, version: Version, files: usize) -> Self {
        Self::Completed {
            package: package.into(),
            version,
            installed_files: files,
            install_path: PathBuf::from("/opt/pm/live"),
            duration: Duration::from_secs(0),
            disk_usage: 0,
        }
    }

    /// Create a basic installation failed event
    pub fn failed(package: impl Into<String>, version: Version, error: impl Into<String>) -> Self {
        Self::Failed {
            package: package.into(),
            version,
            phase: InstallPhase::FileInstallation,
            error: error.into(),
            cleanup_required: false,
        }
    }
}
