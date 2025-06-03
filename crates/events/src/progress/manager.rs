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

//! Thread-safe progress management with event integration

use super::config::ProgressPhase;
use super::tracker::ProgressTracker;
use super::update::ProgressUpdate;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Thread-safe progress tracker manager
#[derive(Debug, Clone)]
pub struct ProgressManager {
    trackers: Arc<Mutex<std::collections::HashMap<String, ProgressTracker>>>,
}

impl ProgressManager {
    /// Create a new progress manager
    pub fn new() -> Self {
        Self {
            trackers: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Create a new progress tracker
    pub fn create_tracker(&self, id: String, operation: String, total: Option<u64>) -> String {
        let tracker = ProgressTracker::new(id.clone(), operation, total);
        let mut trackers = self.trackers.lock().unwrap();
        trackers.insert(id.clone(), tracker);
        id
    }

    /// Create a tracker with phases
    pub fn create_tracker_with_phases(
        &self,
        id: String,
        operation: String,
        total: Option<u64>,
        phases: Vec<ProgressPhase>,
    ) -> String {
        let tracker = ProgressTracker::new(id.clone(), operation, total).with_phases(phases);
        let mut trackers = self.trackers.lock().unwrap();
        trackers.insert(id.clone(), tracker);
        id
    }

    /// Update a tracker's progress
    pub fn update(&self, id: &str, progress: u64) -> Option<ProgressUpdate> {
        let mut trackers = self.trackers.lock().unwrap();
        trackers.get_mut(id).map(|tracker| tracker.update(progress))
    }

    /// Advance a tracker to the next phase
    pub fn next_phase(&self, id: &str) -> Option<usize> {
        let mut trackers = self.trackers.lock().unwrap();
        trackers.get_mut(id).and_then(ProgressTracker::next_phase)
    }

    /// Complete a tracker
    pub fn complete(&self, id: &str) -> Option<Duration> {
        let mut trackers = self.trackers.lock().unwrap();
        if let Some(tracker) = trackers.get_mut(id) {
            let duration = tracker.complete();
            Some(duration)
        } else {
            None
        }
    }

    /// Remove a completed tracker
    #[must_use]
    pub fn remove(&self, id: &str) -> bool {
        let mut trackers = self.trackers.lock().unwrap();
        trackers.remove(id).is_some()
    }

    /// Get current memory usage of all trackers
    #[must_use]
    pub fn total_memory_usage(&self) -> usize {
        let trackers = self.trackers.lock().unwrap();
        trackers.values().map(ProgressTracker::memory_usage).sum()
    }

    /// Get number of active trackers
    #[must_use]
    pub fn active_count(&self) -> usize {
        let trackers = self.trackers.lock().unwrap();
        trackers.len()
    }

    /// Clean up completed trackers to free memory
    #[must_use]
    pub fn cleanup_completed(&self) -> usize {
        let mut trackers = self.trackers.lock().unwrap();
        let initial_count = trackers.len();
        trackers.retain(|_, tracker| !tracker.completed);
        initial_count - trackers.len()
    }

    /// Start a new operation with progress tracking
    pub fn start_operation(
        &self,
        id: &str,
        operation: &str,
        total: Option<u64>,
        phases: Vec<ProgressPhase>,
        _tx: crate::EventSender,
    ) -> String {
        let tracker_id = format!("{}_{}", id, uuid::Uuid::new_v4());
        self.create_tracker_with_phases(tracker_id.clone(), operation.to_string(), total, phases);
        tracker_id
    }

    /// Update progress for an operation
    pub fn update_progress(
        &self,
        id: &str,
        current: u64,
        total: Option<u64>,
        tx: &crate::EventSender,
    ) {
        if let Some(update) = self.update(id, current) {
            // Send progress event
            if let Some(total) = total {
                let _ = tx.send(crate::Event::ProgressUpdated {
                    id: id.to_string(),
                    current,
                    total: Some(total),
                    phase: update.phase,
                    speed: update.speed,
                    eta: update.eta,
                });
            }
        }
    }

    /// Change to a specific phase
    pub fn change_phase(&self, id: &str, _phase: usize, tx: &crate::EventSender) {
        // For now, we'll just advance through phases sequentially
        if let Some(new_phase) = self.next_phase(id) {
            let _ = tx.send(crate::Event::ProgressPhaseChanged {
                id: id.to_string(),
                phase: new_phase,
                phase_name: format!("Phase {}", new_phase),
            });
        }
    }

    /// Complete an operation
    pub fn complete_operation(&self, id: &str, tx: &crate::EventSender) {
        if let Some(duration) = self.complete(id) {
            let _ = tx.send(crate::Event::ProgressCompleted {
                id: id.to_string(),
                duration,
            });
        }
    }
}

impl Default for ProgressManager {
    fn default() -> Self {
        Self::new()
    }
}
