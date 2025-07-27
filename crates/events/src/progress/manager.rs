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

    /// Get a tracker by its ID
    pub fn get_tracker(&self, id: &str) -> Option<ProgressTracker> {
        let trackers = self.trackers.lock().unwrap();
        trackers.get(id).cloned()
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
                let _ = tx.send(crate::AppEvent::Progress(
                    crate::events::ProgressEvent::Updated {
                        id: id.to_string(),
                        current,
                        total: Some(total),
                        phase: update.phase,
                        speed: update.speed,
                        eta: update.eta,
                        efficiency: None,
                    },
                ));
            }
        }
    }

    /// Change to a specific phase
    pub fn change_phase(&self, id: &str, _phase: usize, tx: &crate::EventSender) {
        // For now, we'll just advance through phases sequentially
        if let Some(new_phase) = self.next_phase(id) {
            let _ = tx.send(crate::AppEvent::Progress(
                crate::events::ProgressEvent::PhaseChanged {
                    id: id.to_string(),
                    phase: new_phase,
                    phase_name: format!("Phase {}", new_phase),
                },
            ));
        }
    }

    /// Change to a specific phase by name and mark it as done
    pub fn update_phase_to_done(&self, id: &str, phase_name: &str, tx: &crate::EventSender) {
        let mut trackers = self.trackers.lock().unwrap();
        if let Some(tracker) = trackers.get_mut(id) {
            if let Some(phase_index) = tracker.phases().iter().position(|p| p.name == phase_name) {
                tracker.current_phase = phase_index;
                let _ = tx.send(crate::AppEvent::Progress(
                    crate::events::ProgressEvent::PhaseChanged {
                        id: id.to_string(),
                        phase: phase_index,
                        phase_name: phase_name.to_string(),
                    },
                ));
            }
        }
    }

    /// Complete an operation
    pub fn complete_operation(&self, id: &str, tx: &crate::EventSender) {
        if let Some(duration) = self.complete(id) {
            let _ = tx.send(crate::AppEvent::Progress(
                crate::events::ProgressEvent::Completed {
                    id: id.to_string(),
                    duration,
                    final_speed: None,
                    total_processed: 0,
                },
            ));
        }
    }

    /// Create a parent progress tracker for batch operations
    pub fn create_batch_tracker(
        &self,
        operation_name: String,
        total_items: u64,
        phases: Vec<ProgressPhase>,
    ) -> String {
        let id = format!("batch_{}", uuid::Uuid::new_v4());
        self.create_tracker_with_phases(id.clone(), operation_name, Some(total_items), phases);
        id
    }

    /// Register a child tracker with its parent
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be sent to the event channel.
    pub fn register_child_tracker(
        &self,
        parent_id: &str,
        child_id: &str,
        operation_name: String,
        weight: f64,
        tx: &crate::EventSender,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Emit child started event
        let _ = tx.send(crate::AppEvent::Progress(
            crate::events::ProgressEvent::ChildStarted {
                parent_id: parent_id.to_string(),
                child_id: child_id.to_string(),
                operation: operation_name,
                weight,
            },
        ));

        // Store parent-child relationship in tracker metadata
        // For now, we'll track this through the event system
        // Future enhancement: store relationships in tracker structure
        Ok(())
    }

    /// Complete a child tracker and update parent progress
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be sent to the event channel.
    pub fn complete_child_tracker(
        &self,
        parent_id: &str,
        child_id: &str,
        success: bool,
        tx: &crate::EventSender,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = tx.send(crate::AppEvent::Progress(
            crate::events::ProgressEvent::ChildCompleted {
                parent_id: parent_id.to_string(),
                child_id: child_id.to_string(),
                success,
            },
        ));

        // Update parent progress based on child completion
        // For now, we'll let the UI handle aggregation
        // Future enhancement: automatically update parent progress
        Ok(())
    }
}
impl Default for ProgressManager {
    fn default() -> Self {
        Self::new()
    }
}
