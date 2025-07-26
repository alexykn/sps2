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
//! use sps2_events::{ProgressManager, events::ProgressPhase};
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
//! - Lock-free operations where possible for performance

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
        pub operation_name: String,
        pub total_bytes: Option<u64>,
        pub package_name: Option<String>,
        pub url: String,
    }

    /// Configuration for install progress tracking  
    #[derive(Debug, Clone)]
    pub struct InstallProgressConfig {
        pub operation_name: String,
        pub package_count: u64,
        pub include_dependency_resolution: bool,
    }

    /// Configuration for update/upgrade progress tracking
    #[derive(Debug, Clone)]
    pub struct UpdateProgressConfig {
        pub operation_name: String,
        pub package_count: u64,
        pub is_upgrade: bool, // true for upgrade, false for update
    }

    /// Configuration for uninstall progress tracking
    #[derive(Debug, Clone)]
    pub struct UninstallProgressConfig {
        pub operation_name: String,
        pub package_count: u64,
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
                },
                ProgressPhase {
                    name: "Download".to_string(),
                    weight: 0.9,
                    estimated_duration: None, // Calculated based on speed
                },
                ProgressPhase {
                    name: "Verify".to_string(),
                    weight: 0.05,
                    estimated_duration: Some(Duration::from_secs(1)),
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
                });
            }

            phases.extend_from_slice(&[
                ProgressPhase {
                    name: "Download".to_string(),
                    weight: 0.5,
                    estimated_duration: None,
                },
                ProgressPhase {
                    name: "Validate".to_string(),
                    weight: 0.15,
                    estimated_duration: None,
                },
                ProgressPhase {
                    name: "Stage".to_string(),
                    weight: 0.15,
                    estimated_duration: None,
                },
                ProgressPhase {
                    name: "Commit".to_string(),
                    weight: 0.1,
                    estimated_duration: Some(Duration::from_secs(2)),
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
                },
                ProgressPhase {
                    name: "Download".to_string(),
                    weight: 0.6,
                    estimated_duration: None,
                },
                ProgressPhase {
                    name: "Install".to_string(),
                    weight: 0.3,
                    estimated_duration: None,
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
                },
                ProgressPhase {
                    name: "Remove".to_string(),
                    weight: 0.7,
                    estimated_duration: None,
                },
                ProgressPhase {
                    name: "Cleanup".to_string(),
                    weight: 0.1,
                    estimated_duration: Some(Duration::from_secs(1)),
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
    }
}
