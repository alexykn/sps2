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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::config::ProgressPhase;
    use crate::{EventMessage, EventSender};
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct TestEmitter {
        messages: Mutex<Vec<EventMessage>>,
    }

    #[derive(Debug, Clone, Copy)]
    enum ProgressAssertion<'a> {
        Update { current: u64, phase: usize },
        PhaseChange { phase: usize, name: &'a str },
        Completed,
    }

    impl TestEmitter {
        fn new() -> Self {
            Self::default()
        }

        fn drain(&self) -> Vec<EventMessage> {
            let mut guard = self.messages.lock().expect("messages lock poisoned");
            guard.drain(..).collect()
        }
    }

    impl EventEmitter for TestEmitter {
        fn event_sender(&self) -> Option<&EventSender> {
            None
        }

        fn emit_with_meta(&self, meta: EventMeta, event: AppEvent) {
            let mut guard = self.messages.lock().expect("messages lock poisoned");
            guard.push(EventMessage::new(meta, event));
        }
    }

    fn assert_parent_meta(
        meta: &EventMeta,
        root_event_id: Uuid,
        parent_label: &str,
        is_root: bool,
    ) {
        assert_eq!(
            meta.labels.get("progress_parent").map(String::as_str),
            Some(parent_label)
        );
        if is_root {
            assert_eq!(meta.event_id, root_event_id);
            assert!(meta.parent_id.is_none());
        } else {
            assert_eq!(meta.parent_id, Some(root_event_id));
        }
    }

    fn expect_started_event(
        event: &AppEvent,
        total: Option<u64>,
        expected_phases: &[&str],
        parent_label: &str,
    ) {
        match event {
            AppEvent::Progress(ProgressEvent::Started {
                total: event_total,
                phases,
                parent_id,
                ..
            }) => {
                assert_eq!(*event_total, total);
                let names: Vec<&str> = phases.iter().map(|phase| phase.name.as_str()).collect();
                assert_eq!(names, expected_phases);
                assert_eq!(parent_id.as_deref(), Some(parent_label));
            }
            other => panic!("expected ProgressEvent::Started, got {other:?}"),
        }
    }

    fn expect_update_event(
        event: &AppEvent,
        current: u64,
        total: Option<u64>,
        phase: Option<usize>,
    ) {
        match event {
            AppEvent::Progress(ProgressEvent::Updated {
                current: event_current,
                total: event_total,
                phase: event_phase,
                ..
            }) => {
                assert_eq!(*event_current, current);
                assert_eq!(*event_total, total);
                assert_eq!(*event_phase, phase);
            }
            other => panic!("expected ProgressEvent::Updated, got {other:?}"),
        }
    }

    fn expect_phase_changed_event(event: &AppEvent, phase: usize, name: &str) {
        match event {
            AppEvent::Progress(ProgressEvent::PhaseChanged {
                phase: event_phase,
                phase_name,
                ..
            }) => {
                assert_eq!(*event_phase, phase);
                assert_eq!(phase_name, name);
            }
            other => panic!("expected ProgressEvent::PhaseChanged, got {other:?}"),
        }
    }

    #[test]
    fn started_event_sets_parent_label_and_meta() {
        let manager = ProgressManager::new();
        let tracker_id = manager.create_tracker_with_phases(
            "install".to_string(),
            "install packages".to_string(),
            Some(10),
            vec![],
            None,
        );
        let emitter = TestEmitter::new();

        manager.emit_started(&tracker_id, &emitter, Some("install:pkg"));

        let events = emitter.drain();
        assert_eq!(events.len(), 1);
        let EventMessage { meta, event } = &events[0];
        match event {
            AppEvent::Progress(ProgressEvent::Started { parent_id, .. }) => {
                assert_eq!(parent_id.as_deref(), Some("install:pkg"));
            }
            other => panic!("expected ProgressEvent::Started, got {other:?}"),
        }
        assert_eq!(
            meta.labels.get("progress_parent").map(String::as_str),
            Some("install:pkg")
        );
        assert!(meta.parent_id.is_none());
    }

    #[test]
    fn progress_updates_reference_root_event() {
        let manager = ProgressManager::new();
        let tracker_id = manager.create_tracker_with_phases(
            "install".to_string(),
            "install packages".to_string(),
            Some(5),
            vec![],
            None,
        );
        let emitter = TestEmitter::new();
        manager.emit_started(&tracker_id, &emitter, Some("install:pkg"));
        let root_event_id = manager
            .get_tracker(&tracker_id)
            .expect("tracker")
            .root_event_id();
        emitter.drain();

        manager.update_progress(&tracker_id, 1, Some(5), &emitter);
        let events = emitter.drain();
        assert_eq!(events.len(), 1);
        let EventMessage { meta, event } = &events[0];
        matches!(event, AppEvent::Progress(ProgressEvent::Updated { .. }));
        assert_eq!(meta.parent_id, Some(root_event_id));
        assert_eq!(
            meta.labels.get("progress_parent").map(String::as_str),
            Some("install:pkg")
        );
    }

    #[test]
    fn completion_event_attaches_root_parent() {
        let manager = ProgressManager::new();
        let tracker_id = manager.create_tracker_with_phases(
            "install".to_string(),
            "install packages".to_string(),
            Some(2),
            vec![],
            None,
        );
        let emitter = TestEmitter::new();
        manager.emit_started(&tracker_id, &emitter, Some("install:pkg"));
        let root_event_id = manager
            .get_tracker(&tracker_id)
            .expect("tracker")
            .root_event_id();
        emitter.drain();

        manager.complete_operation(&tracker_id, &emitter);
        let events = emitter.drain();
        assert_eq!(events.len(), 1);
        let EventMessage { meta, event } = &events[0];
        matches!(event, AppEvent::Progress(ProgressEvent::Completed { .. }));
        assert_eq!(meta.parent_id, Some(root_event_id));
        assert_eq!(
            meta.labels.get("progress_parent").map(String::as_str),
            Some("install:pkg")
        );
    }

    #[test]
    fn multi_phase_operation_produces_consistent_event_sequence() {
        let manager = ProgressManager::new();
        let tracker_id = "install-flow".to_string();
        let phases = vec![
            ProgressPhase::new("Resolve", "resolve dependencies"),
            ProgressPhase::new("Fetch", "fetch artifacts"),
            ProgressPhase::new("Install", "link outputs"),
        ];
        manager.create_tracker_with_phases(
            tracker_id.clone(),
            "install packages".to_string(),
            Some(3),
            phases,
            None,
        );
        let emitter = TestEmitter::new();
        let parent_label = "install:root";

        manager.emit_started(&tracker_id, &emitter, Some(parent_label));

        let root_event_id = manager
            .get_tracker(&tracker_id)
            .expect("tracker")
            .root_event_id();

        manager.update_progress(&tracker_id, 1, Some(3), &emitter);
        manager.change_phase(&tracker_id, 1, &emitter);
        manager.update_progress(&tracker_id, 2, Some(3), &emitter);
        manager.update_phase_to_done(&tracker_id, "Install", &emitter);
        manager.update_progress(&tracker_id, 3, Some(3), &emitter);
        manager.complete_operation(&tracker_id, &emitter);

        let mut events = emitter.drain().into_iter();

        let EventMessage { meta, event } = events.next().expect("started event");
        expect_started_event(
            &event,
            Some(3),
            &["Resolve", "Fetch", "Install"],
            parent_label,
        );
        assert_parent_meta(&meta, root_event_id, parent_label, true);

        let expectations = [
            ProgressAssertion::Update {
                current: 1,
                phase: 0,
            },
            ProgressAssertion::PhaseChange {
                phase: 1,
                name: "Fetch",
            },
            ProgressAssertion::Update {
                current: 2,
                phase: 1,
            },
            ProgressAssertion::PhaseChange {
                phase: 2,
                name: "Install",
            },
            ProgressAssertion::Update {
                current: 3,
                phase: 2,
            },
            ProgressAssertion::Completed,
        ];

        for expectation in expectations {
            let EventMessage { meta, event } = events
                .next()
                .unwrap_or_else(|| panic!("missing event for {expectation:?}"));
            match expectation {
                ProgressAssertion::Update { current, phase } => {
                    expect_update_event(&event, current, Some(3), Some(phase));
                }
                ProgressAssertion::PhaseChange { phase, name } => {
                    expect_phase_changed_event(&event, phase, name);
                }
                ProgressAssertion::Completed => {
                    assert!(
                        matches!(event, AppEvent::Progress(ProgressEvent::Completed { .. })),
                        "expected completion event, got {event:?}"
                    );
                }
            }
            assert_parent_meta(&meta, root_event_id, parent_label, false);
        }

        assert!(events.next().is_none(), "unexpected extra events");
    }

    #[test]
    fn change_phase_clamps_index_and_preserves_parent_metadata() {
        let manager = ProgressManager::new();
        let tracker_id = manager.create_tracker_with_phases(
            "batch-job".to_string(),
            "batch operation".to_string(),
            Some(4),
            vec![
                ProgressPhase::new("Stage", "initial staging"),
                ProgressPhase::new("Process", "process work"),
            ],
            None,
        );
        let emitter = TestEmitter::new();
        let parent_label = "batch:parent";

        manager.emit_started(&tracker_id, &emitter, Some(parent_label));
        let root_event_id = manager
            .get_tracker(&tracker_id)
            .expect("tracker")
            .root_event_id();
        emitter.drain();

        manager.change_phase(&tracker_id, 10, &emitter);

        let events = emitter.drain();
        assert_eq!(events.len(), 1);
        let EventMessage { meta, event } = &events[0];
        match event {
            AppEvent::Progress(ProgressEvent::PhaseChanged {
                phase, phase_name, ..
            }) => {
                assert_eq!(*phase, 1);
                assert_eq!(phase_name, "Process");
            }
            other => panic!("expected clamped phase change, got {other:?}"),
        }
        assert_eq!(meta.parent_id, Some(root_event_id));
        assert_eq!(
            meta.labels.get("progress_parent").map(String::as_str),
            Some(parent_label)
        );
    }
}
