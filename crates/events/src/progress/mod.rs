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

//! Sophisticated progress tracking algorithms for enterprise-grade progress reporting
//!
//! This module provides:
//! - Speed calculation with smoothing and outlier detection
//! - Accurate ETA calculations with adaptive windows
//! - Phase-aware progress tracking for multi-stage operations
//! - Memory-efficient data structures (<1KB per tracker)
//!
//! ## Usage Example
//!
//! ```rust
//! use sps2_events::{ProgressManager, config::ProgressPhase};
//! use std::time::Duration;
//!
//! let manager = ProgressManager::new();
//!
//! // Create a tracker for a download operation
//! let id = manager.create_tracker(
//!     "download_jq".to_string(),
//!     "Downloading jq package".to_string(),
//!     Some(1024 * 1024), // 1MB total
//! );
//!
//! // Update progress and get sophisticated metrics
//! if let Some(update) = manager.update(&id, 256 * 1024) {
//!     println!("Progress: {:.1}%", update.percentage().unwrap_or(0.0));
//!     if let Some(speed) = update.format_speed("bytes") {
//!         println!("Speed: {}", speed);
//!     }
//!     if let Some(eta) = update.format_eta() {
//!         println!("ETA: {}", eta);
//!     }
//! }
//!
//! // For multi-phase operations
//! let phases = vec![
//!     ProgressPhase {
//!         name: "Download".to_string(),
//!         weight: 0.7, // 70% of total work
//!         description: Some("Downloading package files".to_string()),
//!     },
//!     ProgressPhase {
//!         name: "Extract".to_string(),
//!         weight: 0.3, // 30% of total work
//!         description: Some("Extracting package contents".to_string()),
//!     },
//! ];
//!
//! let phased_id = manager.create_tracker_with_phases(
//!     "install_package".to_string(),
//!     "Installing package".to_string(),
//!     Some(1000),
//!     phases,
//! );
//!
//! // Progress through phases
//! manager.update(&phased_id, 500); // 50% through download phase
//! manager.next_phase(&phased_id);   // Move to extract phase
//! manager.update(&phased_id, 800); // Continue in extract phase
//!
//! manager.complete(&phased_id);
//! manager.cleanup_completed();
//! ```
//!
//! ## Standardized Progress Patterns
//!
//! The progress system provides pre-configured patterns for common operations:
//!
//! ### Download Operations
//! ```rust
//! use sps2_events::{ProgressManager, patterns::DownloadProgressConfig};
//!
//! let manager = ProgressManager::new();
//! let config = DownloadProgressConfig {
//!     operation_name: "Downloading package".to_string(),
//!     total_bytes: Some(1024 * 1024), // 1MB
//!     package_name: Some("jq".to_string()),
//!     url: "https://example.com/jq.tar.gz".to_string(),
//! };
//! let progress_id = manager.create_download_tracker(&config);
//!
//! // Progress through phases automatically:
//! // 1. Connect (5%) - Network connection establishment
//! // 2. Download (90%) - File transfer with speed/ETA calculation  
//! // 3. Verify (5%) - Hash verification
//! ```
//!
//! ### Install Operations
//! ```rust
//! use sps2_events::{ProgressManager, patterns::InstallProgressConfig};
//!
//! let config = InstallProgressConfig {
//!     operation_name: "Installing packages".to_string(),
//!     package_count: 5,
//!     include_dependency_resolution: true,
//! };
//! let progress_id = manager.create_install_tracker(config);
//!
//! // Phases: Resolve (10%) → Download (50%) → Validate (15%) → Stage (15%) → Commit (10%)
//! ```
//!
//! ### Update/Upgrade Operations
//! ```rust
//! use sps2_events::{ProgressManager, patterns::UpdateProgressConfig};
//!
//! let config = UpdateProgressConfig {
//!     operation_name: "Updating packages".to_string(),
//!     package_count: 3,
//!     is_upgrade: false, // true for upgrade, false for update
//! };
//! let progress_id = manager.create_update_tracker(config);
//!
//! // Phases: Check (10%) → Download (60%) → Install (30%)
//! ```
//!
//! ### Uninstall Operations
//! ```rust
//! use sps2_events::{ProgressManager, patterns::UninstallProgressConfig};
//!
//! let config = UninstallProgressConfig {
//!     operation_name: "Uninstalling packages".to_string(),
//!     package_count: 2,
//! };
//! let progress_id = manager.create_uninstall_tracker(config);
//!
//! // Phases: Analyze (20%) → Remove (70%) → Cleanup (10%)
//! ```
//!
//! ### Parent-Child Progress Coordination
//! ```rust
//! // Create parent tracker for batch operation
//! let parent_id = manager.create_batch_tracker(
//!     "Installing multiple packages".to_string(),
//!     5, // total packages
//!     vec![], // phases handled by children
//! );
//!
//! // Register child trackers
//! for (i, package) in packages.iter().enumerate() {
//!     let child_id = format!("{parent_id}-package-{i}");
//!     let weight = 1.0 / packages.len() as f64; // Equal weight
//!     
//!     manager.register_child_tracker(
//!         &parent_id, &child_id,
//!         format!("Installing {}", package.name),
//!         weight, &tx
//!     )?;
//! }
//! ```
//!
//! ## Best Practices
//!
//! ### Progress ID Generation
//! - IDs are automatically generated with UUIDs for uniqueness
//! - Use descriptive prefixes: `download_`, `install_`, `update_`, `vulndb_`, etc.
//! - For parent-child relationships: `{parent_id}-{operation}-{index}`
//!
//! ### Memory Management
//! - Trackers use <1KB memory each with fixed-size ring buffers
//! - Call `manager.cleanup_completed()` periodically to free memory
//! - Automatic cleanup removes trackers after completion
//! - Use `manager.total_memory_usage()` to monitor memory consumption
//!
//! ### Phase Weight Guidelines
//! - Weights should sum to 1.0 for accurate progress calculation
//! - Network operations: Connect (5%), Transfer (90%), Verify (5%)
//! - Install operations: Download (50%), Process (35%), Commit (15%)
//! - Use `None` for `estimated_duration` when time varies significantly
//! - Heavier phases should have proportionally larger weights
//!
//! ### Error Handling
//! ```rust
//! // Always complete or fail trackers to prevent memory leaks
//! match operation_result {
//!     Ok(_) => manager.complete_operation(&progress_id, &tx),
//!     Err(e) => {
//!         tx.emit(AppEvent::Progress(ProgressEvent::Failed {
//!             id: progress_id,
//!             error: e.to_string(),
//!             completed_items: current_progress,
//!             partial_duration: start_time.elapsed(),
//!         }));
//!     }
//! }
//! ```
//!
//! ### Integration Guidelines
//! - Use standardized patterns (`create_*_tracker`) for consistency
//! - Emit domain-specific events alongside progress events
//! - Update progress regularly during long operations
//! - Clean up trackers after completion to prevent memory leaks
//! - Use parent-child coordination for complex multi-step operations
//!
//! ## Algorithm Details
//!
//! ### Speed Calculation
//! - Uses a configurable sliding window (default: 10 samples)
//! - Automatically detects and filters outliers (>2σ from mean)
//! - Applies exponential moving average for trend emphasis
//! - Handles network jitter and temporary slowdowns gracefully
//!
//! ### ETA Calculation
//! - Combines multiple estimation methods with weighted averages:
//!   - Simple linear projection (40% weight)
//!   - Phase-aware estimation (30% weight)
//!   - Trend-aware projection (30% weight)
//! - Adapts to acceleration/deceleration patterns
//! - Accounts for multi-phase operations with different characteristics
//!
//! ### Memory Efficiency
//! - Fixed-size ring buffers prevent unbounded growth
//! - Automatic pruning of old samples (max 1000 per tracker)
//! - Compact data structures optimized for <1KB per tracker
//! - Minimize locking overhead; manager uses a Mutex internally

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

            self.create_tracker_with_phases(id.clone(), operation, config.total_bytes, phases);
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
            );
            id
        }
    }
}
