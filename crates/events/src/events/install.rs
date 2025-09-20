use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::path::PathBuf;
use std::time::Duration;

/// Installation domain events consumed by the CLI and guard rails
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
        phase: Option<String>,
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

    /// Package staging completed
    StagingCompleted {
        package: String,
        version: Version,
        files_staged: usize,
        staging_size: u64,
        staging_path: PathBuf,
    },

    /// Post-installation validation started
    ValidationStarted {
        package: String,
        version: Version,
        validation_checks: Vec<String>,
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

    /// Batch installation completed
    BatchCompleted {
        operation_id: String,
        successful_packages: Vec<String>,
        failed_packages: Vec<(String, String)>, // (package, error)
        total_duration: Duration,
        total_disk_usage: u64,
    },
}
