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
use crate::EventEmitter;
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
        if let Ok(mut trackers) = self.trackers.lock() {
            trackers.insert(id.clone(), tracker);
        }
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
        if let Ok(mut trackers) = self.trackers.lock() {
            trackers.insert(id.clone(), tracker);
        }
        id
    }

    /// Get a tracker by its ID
    pub fn get_tracker(&self, id: &str) -> Option<ProgressTracker> {
        if let Ok(trackers) = self.trackers.lock() {
            trackers.get(id).cloned()
        } else {
            None
        }
    }

    /// Update a tracker's progress
    pub fn update(&self, id: &str, progress: u64) -> Option<ProgressUpdate> {
        if let Ok(mut trackers) = self.trackers.lock() {
            trackers.get_mut(id).map(|tracker| tracker.update(progress))
        } else {
            None
        }
    }

    /// Advance a tracker to the next phase
    pub fn next_phase(&self, id: &str) -> Option<usize> {
        if let Ok(mut trackers) = self.trackers.lock() {
            trackers.get_mut(id).and_then(ProgressTracker::next_phase)
        } else {
            None
        }
    }

    /// Complete a tracker
    pub fn complete(&self, id: &str) -> Option<Duration> {
        if let Ok(mut trackers) = self.trackers.lock() {
            if let Some(tracker) = trackers.get_mut(id) {
                let duration = tracker.complete();
                Some(duration)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Remove a completed tracker
    #[must_use]
    pub fn remove(&self, id: &str) -> bool {
        if let Ok(mut trackers) = self.trackers.lock() {
            trackers.remove(id).is_some()
        } else {
            false
        }
    }

    /// Get current memory usage of all trackers
    #[must_use]
    pub fn total_memory_usage(&self) -> usize {
        if let Ok(trackers) = self.trackers.lock() {
            trackers.values().map(ProgressTracker::memory_usage).sum()
        } else {
            0
        }
    }

    /// Get number of active trackers
    #[must_use]
    pub fn active_count(&self) -> usize {
        if let Ok(trackers) = self.trackers.lock() {
            trackers.len()
        } else {
            0
        }
    }

    /// Clean up completed trackers to free memory
    #[must_use]
    pub fn cleanup_completed(&self) -> usize {
        if let Ok(mut trackers) = self.trackers.lock() {
            let initial_count = trackers.len();
            trackers.retain(|_, tracker| !tracker.completed);
            initial_count - trackers.len()
        } else {
            0
        }
    }

    /// Start a new operation with progress tracking
    pub fn start_operation(
        &self,
        id: &str,
        operation: &str,
        total: Option<u64>,
        phases: Vec<ProgressPhase>,
        tx: &crate::EventSender,
    ) -> String {
        let tracker_id = format!("{}_{}", id, uuid::Uuid::new_v4());
        let phases_clone = phases.clone();
        self.create_tracker_with_phases(tracker_id.clone(), operation.to_string(), total, phases);
        // Emit a Started event for this operation
        let () = tx.emit(crate::AppEvent::Progress(
            crate::events::ProgressEvent::Started {
                id: tracker_id.clone(),
                operation: operation.to_string(),
                total,
                phases: phases_clone,
                parent_id: None,
            },
        ));
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
            // Send progress event regardless of whether total is known
            let () = tx.emit(crate::AppEvent::Progress(
                crate::events::ProgressEvent::Updated {
                    id: id.to_string(),
                    current,
                    total,
                    phase: update.phase,
                    speed: update.speed,
                    eta: update.eta,
                    efficiency: None,
                },
            ));
        }
    }

    /// Change to a specific phase
    pub fn change_phase(&self, id: &str, phase_index: usize, tx: &crate::EventSender) {
        // Set to specific phase index if available
        if let Ok(mut trackers) = self.trackers.lock() {
            if let Some(tracker) = trackers.get_mut(id) {
                let clamped = phase_index.min(tracker.phases.len().saturating_sub(1));
                tracker.current_phase = clamped;
                let name = tracker
                    .phases()
                    .get(clamped)
                    .map_or_else(|| format!("Phase {}", clamped), |p| p.name.clone());
                let () = tx.emit(crate::AppEvent::Progress(
                    crate::events::ProgressEvent::PhaseChanged {
                        id: id.to_string(),
                        phase: clamped,
                        phase_name: name,
                    },
                ));
            }
        }
    }

    /// Change to a specific phase by name and mark it as done
    pub fn update_phase_to_done(&self, id: &str, phase_name: &str, tx: &crate::EventSender) {
        let mut trackers = self.trackers.lock().unwrap();
        if let Some(tracker) = trackers.get_mut(id) {
            if let Some(phase_index) = tracker.phases().iter().position(|p| p.name == phase_name) {
                tracker.current_phase = phase_index;
                let () = tx.emit(crate::AppEvent::Progress(
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
            let () = tx.emit(crate::AppEvent::Progress(
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
    ) {
        // Emit child started event
        let () = tx.emit(crate::AppEvent::Progress(
            crate::events::ProgressEvent::ChildStarted {
                parent_id: parent_id.to_string(),
                child_id: child_id.to_string(),
                operation: operation_name,
                weight,
            },
        ));
        // Fire-and-forget; UI aggregates parent/child via events.
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
    ) {
        let () = tx.emit(crate::AppEvent::Progress(
            crate::events::ProgressEvent::ChildCompleted {
                parent_id: parent_id.to_string(),
                child_id: child_id.to_string(),
                success,
            },
        ));
        // Fire-and-forget.
    }
}
impl Default for ProgressManager {
    fn default() -> Self {
        Self::new()
    }
}
