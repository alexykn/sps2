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

//! Speed calculation algorithms with smoothing and outlier detection

use super::config::TrendDirection;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Sample point for speed calculation
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // Fields used for potential future analytics
pub(crate) struct SpeedSample {
    /// Timestamp when sample was taken
    pub timestamp: Instant,
    /// Total bytes/items processed at this time
    pub progress: u64,
    /// Time delta since last sample
    pub delta_time: Duration,
    /// Progress delta since last sample
    pub delta_progress: u64,
    /// Instantaneous speed for this sample
    pub speed: f64,
}

/// Efficient ring buffer for speed samples with automatic pruning

#[derive(Debug, Clone)]
pub(crate) struct SpeedBuffer {
    /// Fixed-size ring buffer for recent samples
    pub samples: VecDeque<SpeedSample>,
    /// Maximum number of samples to keep
    max_size: usize,
    /// Sum of speeds for quick average calculation
    speed_sum: f64,
    /// Last recorded progress value
    last_progress: u64,
    /// Last sample timestamp
    last_timestamp: Instant,
}

impl SpeedBuffer {
    pub fn new(max_size: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(max_size),
            max_size,
            speed_sum: 0.0,
            last_progress: 0,
            last_timestamp: Instant::now(),
        }
    }

    /// Add a new sample, calculating speed and managing buffer size
    pub fn add_sample(&mut self, progress: u64, timestamp: Instant) -> Option<f64> {
        let delta_time = timestamp.duration_since(self.last_timestamp);
        let delta_progress = progress.saturating_sub(self.last_progress);

        // Avoid division by zero and very small time deltas
        if delta_time.as_nanos() < 100_000 {
            // Less than 0.1ms, ignore this sample
            return None;
        }

        // Calculate instantaneous speed (units per second)
        #[allow(clippy::cast_precision_loss)] // Precision loss acceptable for speed calculation
        let speed = delta_progress as f64 / delta_time.as_secs_f64();

        let sample = SpeedSample {
            timestamp,
            progress,
            delta_time,
            delta_progress,
            speed,
        };

        // Remove oldest sample if at capacity
        if self.samples.len() >= self.max_size {
            if let Some(old_sample) = self.samples.pop_front() {
                self.speed_sum -= old_sample.speed;
            }
        }

        // Add new sample
        self.samples.push_back(sample);
        self.speed_sum += speed;

        // Update state
        self.last_progress = progress;
        self.last_timestamp = timestamp;

        Some(speed)
    }

    /// Calculate smoothed speed with outlier detection
    pub fn calculate_smoothed_speed(&self, outlier_threshold: f64) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }

        if self.samples.len() == 1 {
            return Some(self.samples[0].speed);
        }

        // Calculate mean and standard deviation for outlier detection
        let mean = self.speed_sum / self.samples.len() as f64;
        let variance = self
            .samples
            .iter()
            .map(|s| (s.speed - mean).powi(2))
            .sum::<f64>()
            / self.samples.len() as f64;
        let std_dev = variance.sqrt();

        // Filter outliers and calculate smoothed average
        let mut valid_speeds = Vec::new();
        for sample in &self.samples {
            // Reject samples more than threshold * std_dev from mean
            if (sample.speed - mean).abs() <= outlier_threshold * std_dev {
                valid_speeds.push(sample.speed);
            }
        }

        if valid_speeds.is_empty() {
            // All samples were outliers, fall back to simple average
            Some(mean)
        } else {
            // Calculate weighted average with recent samples having more weight
            let mut weighted_sum = 0.0;
            let mut weight_sum = 0.0;

            for (i, &speed) in valid_speeds.iter().enumerate() {
                // Linear weighting: newer samples get higher weight
                let weight = 1.0 + i as f64 / valid_speeds.len() as f64;
                weighted_sum += speed * weight;
                weight_sum += weight;
            }

            Some(weighted_sum / weight_sum)
        }
    }

    /// Calculate exponential moving average for trend analysis
    #[allow(dead_code)] // Reserved for future enhanced ETA calculations
    pub fn calculate_ema(&self, alpha: f64) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }

        let mut ema = self.samples[0].speed;
        for sample in self.samples.iter().skip(1) {
            ema = alpha * sample.speed + (1.0 - alpha) * ema;
        }

        Some(ema)
    }

    /// Get recent trend direction (acceleration/deceleration)
    pub fn get_trend(&self) -> TrendDirection {
        if self.samples.len() < 3 {
            return TrendDirection::Stable;
        }

        let recent_count = (self.samples.len() / 3).max(2);
        let recent_samples: Vec<_> = self.samples.iter().rev().take(recent_count).collect();

        // Calculate linear regression slope for recent samples
        let n = recent_samples.len() as f64;
        let sum_x: f64 = (0..recent_samples.len()).map(|i| i as f64).sum();
        let sum_y: f64 = recent_samples.iter().map(|s| s.speed).sum();
        let sum_x_y: f64 = recent_samples
            .iter()
            .enumerate()
            .map(|(i, s)| i as f64 * s.speed)
            .sum();
        let sum_x_squared: f64 = (0..recent_samples.len()).map(|i| (i as f64).powi(2)).sum();

        let slope = (n * sum_x_y - sum_x * sum_y) / (n * sum_x_squared - sum_x.powi(2));

        // Classify trend based on slope
        if slope > 0.1 {
            TrendDirection::Accelerating
        } else if slope < -0.1 {
            TrendDirection::Decelerating
        } else {
            TrendDirection::Stable
        }
    }
}
