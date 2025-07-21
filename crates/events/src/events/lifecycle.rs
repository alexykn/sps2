use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::time::Duration;

/// Package lifecycle events for install/uninstall/update operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LifecycleEvent {
    /// Package resolution started
    ResolutionStarted {
        requested_packages: Vec<String>,
        operation: String, // "install", "update", "upgrade"
    },

    /// Package resolution progress
    ResolutionProgress {
        resolved_packages: usize,
        total_packages: usize,
        current_package: Option<String>,
    },

    /// Package resolution completed
    ResolutionCompleted {
        resolved_packages: usize,
        dependency_count: usize,
        conflicts_detected: usize,
    },

    /// Package acquisition started
    AcquisitionStarted {
        package: String,
        version: Version,
        source: String, // "download", "local", "cache"
    },

    /// Package acquisition progress
    AcquisitionProgress {
        package: String,
        bytes_processed: u64,
        total_bytes: Option<u64>,
    },

    /// Package acquisition completed
    AcquisitionCompleted {
        package: String,
        version: Version,
        source: String,
        size: u64,
    },

    /// Package acquisition failed
    AcquisitionFailed {
        package: String,
        version: Version,
        error: String,
        retry_available: bool,
    },

    /// Package validation started
    ValidationStarted {
        package: String,
        version: Version,
        validation_types: Vec<String>, // "signature", "checksum", "format", etc.
    },

    /// Package validation progress
    ValidationProgress {
        package: String,
        checks_completed: usize,
        total_checks: usize,
        current_check: String,
    },

    /// Package validation completed
    ValidationCompleted {
        package: String,
        version: Version,
        checks_passed: usize,
        warnings: usize,
    },

    /// Package validation failed
    ValidationFailed {
        package: String,
        version: Version,
        failed_check: String,
        error: String,
        can_override: bool,
    },

    /// Package staging started
    StagingStarted {
        package: String,
        version: Version,
        staging_path: String,
    },

    /// Package staging progress
    StagingProgress {
        package: String,
        files_extracted: usize,
        total_files: Option<usize>,
        current_file: Option<String>,
    },

    /// Package staging completed
    StagingCompleted {
        package: String,
        version: Version,
        files_staged: usize,
        staging_size: u64,
    },

    /// Package staging failed
    StagingFailed {
        package: String,
        version: Version,
        error: String,
        partial_cleanup_required: bool,
    },

    /// Package installation preparation
    InstallationPreparing {
        package: String,
        version: Version,
        install_path: String,
    },

    /// Package installation executing
    InstallationExecuting {
        package: String,
        version: Version,
        phase: String, // "files", "links", "metadata", "permissions"
    },

    /// Package installation completed
    InstallationCompleted {
        package: String,
        version: Version,
        installed_files: usize,
        install_path: String,
    },

    /// Package installation failed
    InstallationFailed {
        package: String,
        version: Version,
        phase: String,
        error: String,
        cleanup_required: bool,
    },

    /// Package removal preparation
    RemovalPreparing {
        package: String,
        version: Version,
        dependent_check: bool,
    },

    /// Package removal validation
    RemovalValidating {
        package: String,
        version: Version,
        dependents_found: usize,
        safe_to_remove: bool,
    },

    /// Package removal executing
    RemovalExecuting {
        package: String,
        version: Version,
        files_to_remove: usize,
    },

    /// Package removal progress
    RemovalProgress {
        package: String,
        files_removed: usize,
        total_files: usize,
    },

    /// Package removal completed
    RemovalCompleted {
        package: String,
        version: Version,
        files_removed: usize,
        space_freed: u64,
    },

    /// Package removal failed
    RemovalFailed {
        package: String,
        version: Version,
        error: String,
        partial_removal: bool,
    },

    /// Operation batch started
    OperationBatchStarted {
        operation: String,
        packages: Vec<String>,
        estimated_duration: Option<Duration>,
    },

    /// Operation batch progress
    OperationBatchProgress {
        operation: String,
        completed_packages: usize,
        total_packages: usize,
        current_package: Option<String>,
        current_stage: String,
    },

    /// Operation batch completed
    OperationBatchCompleted {
        operation: String,
        successful_packages: Vec<String>,
        failed_packages: Vec<String>,
        total_duration: Duration,
    },

    /// Operation batch failed
    OperationBatchFailed {
        operation: String,
        error: String,
        completed_packages: Vec<String>,
        rollback_initiated: bool,
    },

    /// Dependency resolution for package
    DependencyResolving {
        package: String,
        count: usize,
    },

    /// Dependency resolved
    DependencyResolved {
        package: String,
        version: Version,
        count: usize,
    },

    /// Dependency conflict detected
    DependencyConflictDetected {
        conflicting_packages: Vec<(String, String)>,
        message: String,
    },

    /// Dependency conflict suggestions
    DependencyConflictSuggestions {
        suggestions: Vec<String>,
    },
}