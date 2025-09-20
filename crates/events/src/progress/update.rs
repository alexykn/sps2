//! Progress update and formatting utilities

use super::config::TrendDirection;
use std::time::Duration;

/// Result of a progress update with calculated metrics
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    /// Tracker ID
    pub id: String,
    /// Current progress
    pub progress: u64,
    /// Total amount of work
    pub total: Option<u64>,
    /// Current phase index
    pub phase: Option<usize>,
    /// Smoothed speed (units per second)
    pub speed: Option<f64>,
    /// Estimated time to completion
    pub eta: Option<Duration>,
    /// Speed trend direction
    pub trend: TrendDirection,
}

impl ProgressUpdate {
    /// Get progress as a percentage (0.0-100.0)
    #[must_use]
    pub fn percentage(&self) -> Option<f64> {
        if let Some(total) = self.total {
            if total > 0 {
                Some((self.progress as f64 / total as f64) * 100.0)
            } else {
                Some(100.0)
            }
        } else {
            None
        }
    }

    /// Format speed in human-readable units
    #[must_use]
    pub fn format_speed(&self, unit: &str) -> Option<String> {
        self.speed.map(|speed| {
            if speed > 1_000_000.0 {
                format!("{:.1}M {unit}/s", speed / 1_000_000.0)
            } else if speed > 1_000.0 {
                format!("{:.1}K {unit}/s", speed / 1_000.0)
            } else {
                format!("{:.1} {unit}/s", speed)
            }
        })
    }

    /// Format ETA in human-readable format
    #[must_use]
    pub fn format_eta(&self) -> Option<String> {
        self.eta.map(|eta| {
            let total_seconds = eta.as_secs();
            if total_seconds > 3600 {
                let hours = total_seconds / 3600;
                let minutes = (total_seconds % 3600) / 60;
                format!("{hours}h {minutes}m")
            } else if total_seconds > 60 {
                let minutes = total_seconds / 60;
                let seconds = total_seconds % 60;
                format!("{minutes}m {seconds}s")
            } else {
                format!("{total_seconds}s")
            }
        })
    }
}
