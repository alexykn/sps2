#![deny(clippy::pedantic, unsafe_code)]
#![allow(
    clippy::module_name_repetitions,
    clippy::cast_precision_loss,        // Mathematical calculations require f64
    clippy::cast_possible_truncation,   // Intentional for progress calculations
    clippy::cast_sign_loss,            // Weights are always positive
    clippy::similar_names,              // Mathematical variable naming is clear
    clippy::missing_panics_doc,         // Mutex::lock panics are documented as safe
    clippy::must_use_candidate,         // Many builder methods are self-evident
    clippy::uninlined_format_args       // Format args are clear in context
)]

// Module declarations
pub mod config;
pub mod manager;
pub mod speed;
pub mod tracker;
pub mod update;

// Public re-exports for main API
pub use config::{ProgressConfig, ProgressPhase, TrendDirection};
pub use manager::ProgressManager;
pub use tracker::ProgressTracker;
pub use update::ProgressUpdate;

/// Standardized progress patterns for user-facing operations
pub mod patterns {
    use super::config::ProgressPhase;
    use super::ProgressManager;
    use std::time::Duration;

    /// Configuration for download progress tracking
    #[derive(Debug, Clone)]
    pub struct DownloadProgressConfig {
        /// Human-readable operation description (e.g., "Downloading jq package")
        pub operation_name: String,

        /// Total bytes to download (enables percentage calculation and ETA)
        /// Set to None for unknown size downloads
        pub total_bytes: Option<u64>,

        /// Package name for display purposes (optional)
        /// Used in progress messages: "Downloading {`package_name`}"
        pub package_name: Option<String>,

        /// Source URL for debugging and logging
        pub url: String,
    }

    /// Configuration for install progress tracking
    #[derive(Debug, Clone)]
    pub struct InstallProgressConfig {
        /// Human-readable operation description (e.g., "Installing packages")
        pub operation_name: String,

        /// Number of packages to install (used for progress calculation)
        pub package_count: u64,

        /// Whether to include dependency resolution phase (adds 10% weight)
        /// Set to true for fresh installs, false for pre-resolved packages
        pub include_dependency_resolution: bool,
    }

    /// Configuration for update/upgrade progress tracking
    #[derive(Debug, Clone)]
    pub struct UpdateProgressConfig {
        /// Human-readable operation description (e.g., "Updating packages")
        pub operation_name: String,

        /// Number of packages to update/upgrade (used for progress calculation)
        pub package_count: u64,

        /// Whether this is an upgrade (true) or update (false)
        /// Affects progress messaging and phase weights
        pub is_upgrade: bool,
    }

    /// Configuration for uninstall progress tracking
    #[derive(Debug, Clone)]
    pub struct UninstallProgressConfig {
        /// Human-readable operation description (e.g., "Uninstalling packages")
        pub operation_name: String,

        /// Number of packages to uninstall (used for progress calculation)
        pub package_count: u64,
    }

    /// Configuration for vulnerability database update progress tracking
    #[derive(Debug, Clone)]
    pub struct VulnDbUpdateProgressConfig {
        /// Human-readable operation description (e.g., "Updating vulnerability database")
        pub operation_name: String,

        /// Number of vulnerability sources to update (e.g., NVD, OSV, GitHub = 3)
        pub sources_count: u64,
    }

    impl ProgressManager {
        /// Create standardized download progress tracker
        pub fn create_download_tracker(&self, config: &DownloadProgressConfig) -> String {
            let id = format!("download_{}", uuid::Uuid::new_v4());
            let operation = format!(
                "Downloading {}",
                config.package_name.as_deref().unwrap_or("package")
            );

            let phases = vec![
                ProgressPhase {
                    name: "Connect".to_string(),
                    weight: 0.05,
                    estimated_duration: Some(Duration::from_secs(2)),
                    description: Some("Establishing network connection".to_string()),
                },
                ProgressPhase {
                    name: "Download".to_string(),
                    weight: 0.9,
                    estimated_duration: None, // Calculated based on speed
                    description: Some("Transferring data".to_string()),
                },
                ProgressPhase {
                    name: "Verify".to_string(),
                    weight: 0.05,
                    estimated_duration: Some(Duration::from_secs(1)),
                    description: Some("Verifying checksum/signature".to_string()),
                },
            ];

            self.create_tracker_with_phases(
                id.clone(),
                operation,
                config.total_bytes,
                phases,
                None,
            );
            id
        }

        /// Create standardized install progress tracker
        pub fn create_install_tracker(&self, config: InstallProgressConfig) -> String {
            let id = format!("install_{}", uuid::Uuid::new_v4());

            let mut phases = Vec::new();

            if config.include_dependency_resolution {
                phases.push(ProgressPhase {
                    name: "Resolve".to_string(),
                    weight: 0.1,
                    estimated_duration: Some(Duration::from_secs(5)),
                    description: Some("Resolving dependencies".to_string()),
                });
            }

            phases.extend_from_slice(&[
                ProgressPhase {
                    name: "Download".to_string(),
                    weight: 0.5,
                    estimated_duration: None,
                    description: Some("Downloading packages".to_string()),
                },
                ProgressPhase {
                    name: "Validate".to_string(),
                    weight: 0.15,
                    estimated_duration: None,
                    description: Some("Validating artifacts".to_string()),
                },
                ProgressPhase {
                    name: "Stage".to_string(),
                    weight: 0.15,
                    estimated_duration: None,
                    description: Some("Staging files".to_string()),
                },
                ProgressPhase {
                    name: "Commit".to_string(),
                    weight: 0.1,
                    estimated_duration: Some(Duration::from_secs(2)),
                    description: Some("Committing to live".to_string()),
                },
            ]);

            self.create_tracker_with_phases(
                id.clone(),
                config.operation_name,
                Some(config.package_count),
                phases,
                None,
            );
            id
        }

        /// Create standardized update/upgrade progress tracker
        pub fn create_update_tracker(&self, config: UpdateProgressConfig) -> String {
            let id = format!("update_{}", uuid::Uuid::new_v4());

            let phases = vec![
                ProgressPhase {
                    name: "Check".to_string(),
                    weight: 0.1,
                    estimated_duration: Some(Duration::from_secs(3)),
                    description: Some("Checking for updates".to_string()),
                },
                ProgressPhase {
                    name: "Download".to_string(),
                    weight: 0.6,
                    estimated_duration: None,
                    description: Some("Downloading updates".to_string()),
                },
                ProgressPhase {
                    name: "Install".to_string(),
                    weight: 0.3,
                    estimated_duration: None,
                    description: Some("Installing updates".to_string()),
                },
            ];

            self.create_tracker_with_phases(
                id.clone(),
                config.operation_name,
                Some(config.package_count),
                phases,
                None,
            );
            id
        }

        /// Create standardized uninstall progress tracker
        pub fn create_uninstall_tracker(&self, config: UninstallProgressConfig) -> String {
            let id = format!("uninstall_{}", uuid::Uuid::new_v4());

            let phases = vec![
                ProgressPhase {
                    name: "Analyze".to_string(),
                    weight: 0.2,
                    estimated_duration: Some(Duration::from_secs(2)),
                    description: Some("Analyzing dependencies".to_string()),
                },
                ProgressPhase {
                    name: "Remove".to_string(),
                    weight: 0.7,
                    estimated_duration: None,
                    description: Some("Removing files".to_string()),
                },
                ProgressPhase {
                    name: "Cleanup".to_string(),
                    weight: 0.1,
                    estimated_duration: Some(Duration::from_secs(1)),
                    description: Some("Cleaning up".to_string()),
                },
            ];

            self.create_tracker_with_phases(
                id.clone(),
                config.operation_name,
                Some(config.package_count),
                phases,
                None,
            );
            id
        }

        /// Create standardized vulnerability database update progress tracker
        pub fn create_vulndb_tracker(&self, config: VulnDbUpdateProgressConfig) -> String {
            let id = format!("vulndb_{}", uuid::Uuid::new_v4());

            let phases = vec![
                ProgressPhase {
                    name: "Initialize".to_string(),
                    weight: 0.1,
                    estimated_duration: Some(Duration::from_secs(2)),
                    description: Some("Initializing update".to_string()),
                },
                ProgressPhase {
                    name: "Download".to_string(),
                    weight: 0.8,
                    estimated_duration: None, // Depends on network speed
                    description: Some("Downloading vulnerability data".to_string()),
                },
                ProgressPhase {
                    name: "Process".to_string(),
                    weight: 0.1,
                    estimated_duration: Some(Duration::from_secs(5)),
                    description: Some("Processing data".to_string()),
                },
            ];

            self.create_tracker_with_phases(
                id.clone(),
                config.operation_name,
                Some(config.sources_count),
                phases,
                None,
            );
            id
        }
    }
}
