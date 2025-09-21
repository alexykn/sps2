//! Thread-safe progress management with event integration

use super::config::ProgressPhase;
use super::tracker::ProgressTracker;
use super::update::ProgressUpdate;
use crate::{AppEvent, EventEmitter, EventLevel, EventMeta, ProgressEvent};
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
    pub fn create_tracker(
        &self,
        id: String,
        operation: String,
        total: Option<u64>,
        parent_id: Option<String>,
    ) -> String {
        let tracker = ProgressTracker::new(id.clone(), operation, total, parent_id);
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
        parent_id: Option<String>,
    ) -> String {
        let tracker =
            ProgressTracker::new(id.clone(), operation, total, parent_id).with_phases(phases);
        if let Ok(mut trackers) = self.trackers.lock() {
            trackers.insert(id.clone(), tracker);
        }
        id
    }

    /// Emit a started event for an existing tracker using its stored metadata.
    pub fn emit_started<E: EventEmitter>(&self, id: &str, emitter: &E, parent_id: Option<&str>) {
        let Ok(mut trackers) = self.trackers.lock() else {
            return;
        };
        let Some(tracker) = trackers.get_mut(id) else {
            return;
        };

        if let Some(parent) = parent_id {
            if tracker.parent_id().is_none() {
                tracker.set_parent_id(Some(parent.to_string()));
            }
        }

        let operation = tracker.operation().to_string();
        let total = tracker.total();
        let phases = tracker.phases().to_vec();
        let parent_label = tracker.parent_id().cloned();
        let root_event_id = tracker.root_event_id();

        let event = ProgressEvent::Started {
            id: id.to_string(),
            operation,
            total,
            phases,
            parent_id: parent_label.clone(),
        };
        let app_event = AppEvent::Progress(event);
        let level = EventLevel::from(app_event.log_level());
        let mut meta = EventMeta::new(level, app_event.event_source());
        meta.event_id = root_event_id;
        if let Some(parent_label) = parent_label {
            meta.labels
                .insert("progress_parent".to_string(), parent_label);
        }
        emitter.enrich_event_meta(&app_event, &mut meta);
        emitter.emit_with_meta(meta, app_event);
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
    pub fn start_operation<E: EventEmitter>(
        &self,
        id: &str,
        operation: &str,
        total: Option<u64>,
        phases: Vec<ProgressPhase>,
        emitter: &E,
        parent_id: Option<&str>,
    ) -> String {
        let tracker_id = format!("{}_{}", id, uuid::Uuid::new_v4());
        let parent_string = parent_id.map(str::to_string);
        self.create_tracker_with_phases(
            tracker_id.clone(),
            operation.to_string(),
            total,
            phases,
            parent_string.clone(),
        );
        // Emit a Started event for this operation with metadata linking
        self.emit_started(&tracker_id, emitter, parent_string.as_deref());
        tracker_id
    }

    /// Update progress for an operation
    pub fn update_progress<E: EventEmitter>(
        &self,
        id: &str,
        current: u64,
        total: Option<u64>,
        emitter: &E,
    ) {
        let Ok(mut trackers) = self.trackers.lock() else {
            return;
        };
        let Some(tracker) = trackers.get_mut(id) else {
            return;
        };

        let update = tracker.update(current);
        let parent_label = tracker.parent_id().cloned();
        let root_event_id = tracker.root_event_id();

        let event = ProgressEvent::Updated {
            id: id.to_string(),
            current,
            total,
            phase: update.phase,
            speed: update.speed,
            eta: update.eta,
            efficiency: None,
        };
        let app_event = AppEvent::Progress(event);
        let level = EventLevel::from(app_event.log_level());
        let mut meta = EventMeta::new(level, app_event.event_source());
        meta.parent_id = Some(root_event_id);
        if let Some(parent_label) = parent_label {
            meta.labels
                .insert("progress_parent".to_string(), parent_label);
        }
        emitter.enrich_event_meta(&app_event, &mut meta);
        emitter.emit_with_meta(meta, app_event);
    }

    /// Change to a specific phase
    pub fn change_phase<E: EventEmitter>(&self, id: &str, phase_index: usize, emitter: &E) {
        // Set to specific phase index if available
        let Ok(mut trackers) = self.trackers.lock() else {
            return;
        };
        let Some(tracker) = trackers.get_mut(id) else {
            return;
        };

        let clamped = phase_index.min(tracker.phases.len().saturating_sub(1));
        tracker.current_phase = clamped;
        let phase_name = tracker
            .phases()
            .get(clamped)
            .map_or_else(|| format!("Phase {}", clamped), |p| p.name.clone());
        let parent_label = tracker.parent_id().cloned();
        let root_event_id = tracker.root_event_id();

        let event = ProgressEvent::PhaseChanged {
            id: id.to_string(),
            phase: clamped,
            phase_name,
        };
        let app_event = AppEvent::Progress(event);
        let level = EventLevel::from(app_event.log_level());
        let mut meta = EventMeta::new(level, app_event.event_source());
        meta.parent_id = Some(root_event_id);
        if let Some(parent_label) = parent_label {
            meta.labels
                .insert("progress_parent".to_string(), parent_label);
        }
        emitter.enrich_event_meta(&app_event, &mut meta);
        emitter.emit_with_meta(meta, app_event);
    }

    /// Change to a specific phase by name and mark it as done
    pub fn update_phase_to_done<E: EventEmitter>(&self, id: &str, phase_name: &str, emitter: &E) {
        let Ok(mut trackers) = self.trackers.lock() else {
            return;
        };
        let Some(tracker) = trackers.get_mut(id) else {
            return;
        };
        let Some(phase_index) = tracker.phases().iter().position(|p| p.name == phase_name) else {
            return;
        };
        tracker.current_phase = phase_index;
        let parent_label = tracker.parent_id().cloned();
        let root_event_id = tracker.root_event_id();

        let event = ProgressEvent::PhaseChanged {
            id: id.to_string(),
            phase: phase_index,
            phase_name: phase_name.to_string(),
        };
        let app_event = AppEvent::Progress(event);
        let level = EventLevel::from(app_event.log_level());
        let mut meta = EventMeta::new(level, app_event.event_source());
        meta.parent_id = Some(root_event_id);
        if let Some(parent_label) = parent_label {
            meta.labels
                .insert("progress_parent".to_string(), parent_label);
        }
        emitter.enrich_event_meta(&app_event, &mut meta);
        emitter.emit_with_meta(meta, app_event);
    }

    /// Complete an operation
    pub fn complete_operation<E: EventEmitter>(&self, id: &str, emitter: &E) {
        let Ok(mut trackers) = self.trackers.lock() else {
            return;
        };
        let Some(tracker) = trackers.get_mut(id) else {
            return;
        };

        let duration = tracker.complete();
        let parent_label = tracker.parent_id().cloned();
        let root_event_id = tracker.root_event_id();

        let event = ProgressEvent::Completed {
            id: id.to_string(),
            duration,
            final_speed: None,
            total_processed: 0,
        };
        let app_event = AppEvent::Progress(event);
        let level = EventLevel::from(app_event.log_level());
        let mut meta = EventMeta::new(level, app_event.event_source());
        meta.parent_id = Some(root_event_id);
        if let Some(parent_label) = parent_label {
            meta.labels
                .insert("progress_parent".to_string(), parent_label);
        }
        emitter.enrich_event_meta(&app_event, &mut meta);
        emitter.emit_with_meta(meta, app_event);
    }

    /// Create a parent progress tracker for batch operations
    pub fn create_batch_tracker(
        &self,
        operation_name: String,
        total_items: u64,
        phases: Vec<ProgressPhase>,
    ) -> String {
        let id = format!("batch_{}", uuid::Uuid::new_v4());
        self.create_tracker_with_phases(
            id.clone(),
            operation_name,
            Some(total_items),
            phases,
            None,
        );
        id
    }

    /// Register a child tracker with its parent
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be sent to the event channel.
    pub fn register_child_tracker<E: EventEmitter>(
        &self,
        parent_id: &str,
        child_id: &str,
        operation_name: String,
        weight: f64,
        emitter: &E,
    ) {
        // Emit child started event
        emitter.emit(AppEvent::Progress(ProgressEvent::ChildStarted {
            parent_id: parent_id.to_string(),
            child_id: child_id.to_string(),
            operation: operation_name,
            weight,
        }));
        // Fire-and-forget; UI aggregates parent/child via events.
    }

    /// Complete a child tracker and update parent progress
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be sent to the event channel.
    pub fn complete_child_tracker<E: EventEmitter>(
        &self,
        parent_id: &str,
        child_id: &str,
        success: bool,
        emitter: &E,
    ) {
        emitter.emit(AppEvent::Progress(ProgressEvent::ChildCompleted {
            parent_id: parent_id.to_string(),
            child_id: child_id.to_string(),
            success,
        }));
        // Fire-and-forget.
    }
}
impl Default for ProgressManager {
    fn default() -> Self {
        Self::new()
    }
}
