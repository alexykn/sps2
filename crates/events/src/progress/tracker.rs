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

//! Core progress tracking with sophisticated ETA calculations

use super::config::{ProgressConfig, ProgressPhase, TrendDirection};
use super::speed::{SpeedBuffer, SpeedSample};
use super::update::ProgressUpdate;
use std::time::{Duration, Instant};

/// Core progress tracker with sophisticated algorithms
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    /// Unique identifier for this tracker
    id: String,
    /// Human-readable operation name
    operation: String,
    /// Total amount of work (bytes, items, etc.)
    total: Option<u64>,
    /// Current progress
    current: u64,
    /// Phases for multi-stage operations
    pub phases: Vec<ProgressPhase>,
    /// Current active phase
    pub current_phase: usize,
    /// Speed calculation buffer
    speed_buffer: SpeedBuffer,
    /// Configuration for algorithms
    config: ProgressConfig,
    /// When tracking started
    start_time: Instant,
    /// Last update time
    last_update: Instant,
    /// Exponential moving average state
    ema_speed: Option<f64>,
    /// Whether tracker has been completed
    pub(crate) completed: bool,
}

impl ProgressTracker {
    /// Create a new progress tracker
    #[must_use]
    pub fn new(id: String, operation: String, total: Option<u64>) -> Self {
        let config = ProgressConfig::default();
        let now = Instant::now();

        Self {
            id,
            operation,
            total,
            current: 0,
            phases: Vec::new(),
            current_phase: 0,
            speed_buffer: SpeedBuffer::new(config.speed_window_size),
            config,
            start_time: now,
            last_update: now,
            ema_speed: None,
            completed: false,
        }
    }

    /// Create a new progress tracker with custom configuration
    #[must_use]
    pub fn with_config(
        id: String,
        operation: String,
        total: Option<u64>,
        config: ProgressConfig,
    ) -> Self {
        let now = Instant::now();

        Self {
            id,
            operation,
            total,
            current: 0,
            phases: Vec::new(),
            current_phase: 0,
            speed_buffer: SpeedBuffer::new(config.speed_window_size),
            config,
            start_time: now,
            last_update: now,
            ema_speed: None,
            completed: false,
        }
    }

    /// Add phases for multi-stage operations
    #[must_use]
    pub fn with_phases(mut self, phases: Vec<ProgressPhase>) -> Self {
        // Normalize phase weights to sum to 1.0
        let total_weight: f64 = phases.iter().map(|p| p.weight).sum();
        let mut normalized = phases;
        if total_weight > 0.0 {
            for phase in &mut normalized {
                phase.weight /= total_weight;
            }
        }
        self.phases = normalized;
        self
    }

    /// Update progress and calculate metrics
    pub fn update(&mut self, progress: u64) -> ProgressUpdate {
        let now = Instant::now();

        // For tests and first update, always process
        let should_update = now.duration_since(self.last_update) >= self.config.update_interval
            || self.speed_buffer.samples.is_empty();

        if !should_update {
            return ProgressUpdate {
                id: self.id.clone(),
                progress,
                total: self.total,
                phase: if self.phases.is_empty() {
                    None
                } else {
                    Some(self.current_phase)
                },
                speed: self.ema_speed,
                eta: None,
                trend: TrendDirection::Stable,
            };
        }

        self.current = progress;
        self.last_update = now;

        // Add speed sample
        if let Some(instantaneous_speed) = self.speed_buffer.add_sample(progress, now) {
            // Update exponential moving average
            if let Some(current_ema) = self.ema_speed {
                self.ema_speed = Some(
                    self.config.ema_alpha * instantaneous_speed
                        + (1.0 - self.config.ema_alpha) * current_ema,
                );
            } else {
                self.ema_speed = Some(instantaneous_speed);
            }
        }

        // Calculate smoothed speed
        let smoothed_speed = self
            .speed_buffer
            .calculate_smoothed_speed(self.config.outlier_threshold);

        // Calculate ETA using multiple methods and pick the best
        let eta = self.calculate_eta(smoothed_speed);

        // Get trend direction
        let trend = self.speed_buffer.get_trend();

        ProgressUpdate {
            id: self.id.clone(),
            progress,
            total: self.total,
            phase: if self.phases.is_empty() {
                None
            } else {
                Some(self.current_phase)
            },
            speed: smoothed_speed,
            eta,
            trend,
        }
    }

    /// Advance to the next phase
    pub fn next_phase(&mut self) -> Option<usize> {
        if self.current_phase + 1 < self.phases.len() {
            self.current_phase += 1;

            // Reset speed calculations for new phase
            self.speed_buffer = SpeedBuffer::new(self.config.speed_window_size);
            self.ema_speed = None;

            Some(self.current_phase)
        } else {
            None
        }
    }

    /// Mark tracker as completed
    pub fn complete(&mut self) -> Duration {
        self.completed = true;
        self.start_time.elapsed()
    }

    /// Calculate ETA using multiple sophisticated methods
    fn calculate_eta(&self, current_speed: Option<f64>) -> Option<Duration> {
        if self.completed || self.total.is_none() {
            return None;
        }

        let total = self.total?;
        let remaining = total.saturating_sub(self.current);

        if remaining == 0 {
            return Some(Duration::ZERO);
        }

        // Need minimum samples for reliable ETA
        if self.speed_buffer.samples.len() < self.config.min_samples_for_eta {
            return None;
        }

        let speed = current_speed?;

        if speed <= 0.0 {
            return None;
        }

        // Method 1: Simple ETA based on current speed
        let simple_eta = Duration::from_secs_f64(remaining as f64 / speed);

        // Method 2: Phase-aware ETA if we have phases
        let phase_eta = if self.phases.is_empty() {
            simple_eta
        } else {
            self.calculate_phase_aware_eta(remaining, speed)
        };

        // Method 3: Trend-aware ETA
        let trend_eta = self.calculate_trend_aware_eta(remaining, speed);

        // Combine estimates using weighted average
        let estimates = [
            (simple_eta, 0.4), // 40% weight on simple calculation
            (phase_eta, 0.3),  // 30% weight on phase-aware
            (trend_eta, 0.3),  // 30% weight on trend-aware
        ];

        let total_weight: f64 = estimates.iter().map(|(_, w)| w).sum();
        let weighted_sum: f64 = estimates
            .iter()
            .map(|(eta, weight)| eta.as_secs_f64() * weight)
            .sum();

        Some(Duration::from_secs_f64(weighted_sum / total_weight))
    }

    /// Calculate phase-aware ETA considering current phase progress
    fn calculate_phase_aware_eta(&self, remaining: u64, speed: f64) -> Duration {
        if self.phases.is_empty() {
            return Duration::from_secs_f64(remaining as f64 / speed);
        }

        let current_phase = &self.phases[self.current_phase];
        let total = self.total.unwrap_or(0);

        // Calculate how much work is left in current phase
        let phase_start = self
            .phases
            .iter()
            .take(self.current_phase)
            .map(|p| (total as f64 * p.weight) as u64)
            .sum::<u64>();

        let phase_total = (total as f64 * current_phase.weight) as u64;
        let phase_remaining = phase_total.saturating_sub(self.current - phase_start);

        // Calculate remaining work in future phases
        let future_phases_work: u64 = self
            .phases
            .iter()
            .skip(self.current_phase + 1)
            .map(|p| (total as f64 * p.weight) as u64)
            .sum();

        // Estimate time for current phase
        let current_phase_eta = Duration::from_secs_f64(phase_remaining as f64 / speed);

        // Estimate time for future phases (assume same speed)
        let future_phases_eta = Duration::from_secs_f64(future_phases_work as f64 / speed);

        current_phase_eta + future_phases_eta
    }

    /// Calculate trend-aware ETA considering acceleration/deceleration
    fn calculate_trend_aware_eta(&self, remaining: u64, speed: f64) -> Duration {
        let trend = self.speed_buffer.get_trend();

        match trend {
            TrendDirection::Accelerating => {
                // If accelerating, assume speed will continue to increase
                // Use a conservative 10% acceleration factor
                let projected_speed = speed * 1.1;
                Duration::from_secs_f64(remaining as f64 / projected_speed)
            }
            TrendDirection::Decelerating => {
                // If decelerating, assume speed will continue to decrease
                // Use a conservative 10% deceleration factor
                let projected_speed = speed * 0.9;
                Duration::from_secs_f64(remaining as f64 / projected_speed)
            }
            TrendDirection::Stable => {
                // Use current speed as-is
                Duration::from_secs_f64(remaining as f64 / speed)
            }
        }
    }

    /// Get current phase information
    pub fn current_phase(&self) -> Option<&ProgressPhase> {
        self.phases.get(self.current_phase)
    }

    /// Get all phases
    pub fn phases(&self) -> &[ProgressPhase] {
        &self.phases
    }

    /// Get memory usage estimate for this tracker
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        // Base struct size
        let base_size = std::mem::size_of::<Self>();

        // String allocations
        let string_size = self.id.capacity() + self.operation.capacity();

        // Phases vector
        let phases_size = self.phases.capacity() * std::mem::size_of::<ProgressPhase>()
            + self
                .phases
                .iter()
                .map(|p| {
                    p.name.capacity()
                        + p.description
                            .as_ref()
                            .map_or(0, std::string::String::capacity)
                })
                .sum::<usize>();

        // Speed buffer samples
        let samples_size =
            self.speed_buffer.samples.capacity() * std::mem::size_of::<SpeedSample>();

        base_size + string_size + phases_size + samples_size
    }
}
