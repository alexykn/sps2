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
//! use sps2_events::{ProgressManager, ProgressPhase};
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
//!         estimated_duration: None,
//!     },
//!     ProgressPhase {
//!         name: "Extract".to_string(),
//!         weight: 0.3, // 30% of total work
//!         estimated_duration: None,
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
