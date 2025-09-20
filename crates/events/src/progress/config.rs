//! Configuration and core types for progress tracking

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for progress tracking algorithms
#[derive(Debug, Clone)]
pub struct ProgressConfig {
    /// Number of samples for moving average (default: 10)
    pub speed_window_size: usize,
    /// Maximum samples to retain in history (default: 1000)
    pub max_history_samples: usize,
    /// Update frequency for smooth UI (default: 100ms)
    pub update_interval: Duration,
    /// Outlier rejection multiplier (default: 2.0)
    pub outlier_threshold: f64,
    /// Exponential moving average alpha (default: 0.3)
    pub ema_alpha: f64,
    /// Minimum samples needed for reliable ETA (default: 3)
    pub min_samples_for_eta: usize,
}

impl Default for ProgressConfig {
    fn default() -> Self {
        Self {
            speed_window_size: 10,
            max_history_samples: 1000,
            update_interval: Duration::from_millis(100),
            outlier_threshold: 2.0,
            ema_alpha: 0.3,
            min_samples_for_eta: 3,
        }
    }
}

/// A phase in a multi-stage operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressPhase {
    /// Human-readable name of the phase
    pub name: String,
    /// Weight of this phase relative to others (0.0-1.0)
    pub weight: f64,
    /// Optional estimated duration for this phase
    pub estimated_duration: Option<Duration>,
    /// Optional human-readable description of the phase
    pub description: Option<String>,
}

impl ProgressPhase {
    /// Create a new progress phase
    #[must_use]
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            weight: 1.0, // Default equal weight
            estimated_duration: None,
            description: Some(description.to_string()),
        }
    }

    /// Set the weight for this phase
    #[must_use]
    pub fn with_weight(mut self, weight: f64) -> Self {
        self.weight = weight;
        self
    }

    /// Set the estimated duration for this phase
    #[must_use]
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.estimated_duration = Some(duration);
        self
    }
}

/// Direction of speed trend
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendDirection {
    /// Speed is increasing
    Accelerating,
    /// Speed is decreasing
    Decelerating,
    /// Speed is relatively stable
    Stable,
}
