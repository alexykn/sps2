use serde::{Deserialize, Serialize};
use std::time::Duration;
// Use the unified progress phase type from the progress module
use crate::progress::config::ProgressPhase;

/// Progress tracking events with sophisticated algorithms
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProgressEvent {
    /// Progress tracking started
    Started {
        id: String,
        operation: String,
        total: Option<u64>,
        phases: Vec<ProgressPhase>,
        parent_id: Option<String>,
    },

    /// Progress updated with calculated metrics
    Updated {
        id: String,
        current: u64,
        total: Option<u64>,
        phase: Option<usize>,
        speed: Option<f64>,
        eta: Option<Duration>,
        efficiency: Option<f64>,
    },

    /// Progress phase changed
    PhaseChanged {
        id: String,
        phase: usize,
        phase_name: String,
    },

    /// Progress completed successfully
    Completed {
        id: String,
        duration: Duration,
        final_speed: Option<f64>,
        total_processed: u64,
    },

    /// Progress failed
    Failed {
        id: String,
        failure: super::FailureContext,
        completed_items: u64,
        partial_duration: Duration,
    },

    /// Progress paused
    Paused {
        id: String,
        reason: String,
        items_completed: u64,
    },

    /// Progress resumed
    Resumed {
        id: String,
        pause_duration: Duration,
    },

    /// Nested progress - child operation started
    ChildStarted {
        parent_id: String,
        child_id: String,
        operation: String,
        weight: f64, // Contribution to parent progress (0.0-1.0)
    },

    /// Nested progress - child operation completed
    ChildCompleted {
        parent_id: String,
        child_id: String,
        success: bool,
    },
}

impl ProgressEvent {
    /// Create a simple progress started event
    pub fn started(
        id: impl Into<String>,
        operation: impl Into<String>,
        total: Option<u64>,
    ) -> Self {
        Self::Started {
            id: id.into(),
            operation: operation.into(),
            total,
            phases: vec![],
            parent_id: None,
        }
    }

    /// Create a progress started event with phases
    pub fn started_with_phases(
        id: impl Into<String>,
        operation: impl Into<String>,
        total: Option<u64>,
        phases: Vec<ProgressPhase>,
    ) -> Self {
        Self::Started {
            id: id.into(),
            operation: operation.into(),
            total,
            phases,
            parent_id: None,
        }
    }

    /// Create a child progress started event
    pub fn child_started(
        parent_id: impl Into<String>,
        child_id: impl Into<String>,
        operation: impl Into<String>,
        weight: f64,
    ) -> Self {
        Self::ChildStarted {
            parent_id: parent_id.into(),
            child_id: child_id.into(),
            operation: operation.into(),
            weight,
        }
    }

    /// Create a progress update event
    pub fn updated(id: impl Into<String>, current: u64, total: Option<u64>) -> Self {
        Self::Updated {
            id: id.into(),
            current,
            total,
            phase: None,
            speed: None,
            eta: None,
            efficiency: None,
        }
    }

    /// Create a progress completed event
    pub fn completed(id: impl Into<String>, duration: Duration) -> Self {
        Self::Completed {
            id: id.into(),
            duration,
            final_speed: None,
            total_processed: 0,
        }
    }

    /// Create a progress failed event
    pub fn failed(id: impl Into<String>, failure: super::FailureContext) -> Self {
        Self::Failed {
            id: id.into(),
            failure,
            completed_items: 0,
            partial_duration: Duration::from_secs(0),
        }
    }
}
