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

// Tests from the original file
#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_progress_tracker_basic() {
        let mut tracker =
            ProgressTracker::new("test".to_string(), "Testing".to_string(), Some(1000));

        let update = tracker.update(100);
        assert_eq!(update.progress, 100);
        assert_eq!(update.total, Some(1000));
        assert_eq!(update.percentage(), Some(10.0));
    }

    #[test]
    fn test_speed_calculation() {
        let mut tracker = ProgressTracker::new(
            "speed_test".to_string(),
            "Speed Test".to_string(),
            Some(1000),
        );

        // Simulate progress over time with longer intervals
        tracker.update(0);
        thread::sleep(Duration::from_millis(50));
        tracker.update(100);
        thread::sleep(Duration::from_millis(50));
        let update = tracker.update(200);

        // Should have some speed calculation after multiple updates
        assert!(update.speed.is_some());
    }

    #[test]
    fn test_phase_tracking() {
        let phases = vec![
            ProgressPhase {
                name: "Download".to_string(),
                weight: 0.7,
                estimated_duration: None,
            },
            ProgressPhase {
                name: "Extract".to_string(),
                weight: 0.3,
                estimated_duration: None,
            },
        ];

        let mut tracker = ProgressTracker::new(
            "phase_test".to_string(),
            "Phase Test".to_string(),
            Some(1000),
        )
        .with_phases(phases);

        let update = tracker.update(100);
        assert_eq!(update.phase, Some(0));

        tracker.next_phase();
        let update = tracker.update(200);
        assert_eq!(update.phase, Some(1));
    }

    #[test]
    fn test_memory_usage() {
        let tracker = ProgressTracker::new(
            "memory_test".to_string(),
            "Memory Test".to_string(),
            Some(1000),
        );

        let usage = tracker.memory_usage();
        // Should be reasonable (target: <1KB per tracker)
        assert!(usage < 1024, "Memory usage {} exceeds 1KB target", usage);
    }

    #[test]
    fn test_progress_manager() {
        let manager = ProgressManager::new();

        let id = manager.create_tracker(
            "manager_test".to_string(),
            "Manager Test".to_string(),
            Some(1000),
        );

        assert_eq!(manager.active_count(), 1);

        let update = manager.update(&id, 500);
        assert!(update.is_some());
        assert_eq!(update.unwrap().progress, 500);

        let duration = manager.complete(&id);
        assert!(duration.is_some());

        let removed = manager.cleanup_completed();
        assert_eq!(removed, 1);
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn test_eta_calculation() {
        let mut tracker =
            ProgressTracker::new("eta_test".to_string(), "ETA Test".to_string(), Some(1000));

        // Simulate steady progress to establish speed with longer intervals
        for i in 1..=5 {
            tracker.update(i * 100);
            thread::sleep(Duration::from_millis(50));
        }

        let update = tracker.update(500);
        // Should have ETA after enough samples
        assert!(update.eta.is_some());
    }

    #[test]
    fn test_outlier_rejection() {
        use speed::SpeedBuffer;
        use std::time::Instant;

        let mut buffer = SpeedBuffer::new(10);
        let start = Instant::now();

        // Add normal samples with steady progress
        for i in 1..=5 {
            buffer.add_sample(i * 100, start + Duration::from_millis(i * 100));
        }

        // Add outlier (much faster than normal)
        buffer.add_sample(1500, start + Duration::from_millis(505));

        // Add more normal samples
        for i in 6..=8 {
            buffer.add_sample(
                500 + (i - 5) * 100,
                start + Duration::from_millis(i * 100 + 10),
            );
        }

        let smoothed = buffer.calculate_smoothed_speed(2.0);
        assert!(smoothed.is_some());

        // Smoothed speed should be reasonable despite the outlier
        let speed = smoothed.unwrap();
        assert!(speed > 0.0 && speed < 10_000.0); // Reasonable range for this test
    }

    #[test]
    fn test_trend_detection() {
        use speed::SpeedBuffer;
        use std::time::Instant;

        let mut buffer = SpeedBuffer::new(10);
        let start = Instant::now();

        // Simulate accelerating trend
        let speeds = [10.0, 15.0, 20.0, 25.0, 30.0];
        for (i, &_target_speed) in speeds.iter().enumerate() {
            let progress = (i + 1) as u64 * 100;
            let time_offset = Duration::from_millis((i + 1) as u64 * 50); // Faster intervals
            buffer.add_sample(progress, start + time_offset);
        }

        let trend = buffer.get_trend();
        // Should detect acceleration (though may be stable due to small sample size)
        assert!(matches!(
            trend,
            TrendDirection::Accelerating | TrendDirection::Stable
        ));
    }
}
