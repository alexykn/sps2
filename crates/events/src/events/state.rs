use serde::{Deserialize, Serialize};
use sps2_types::StateId;
use std::time::Duration;

/// State management milestones for atomic operations and cleanup
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StateEvent {
    /// A state transition started.
    TransitionStarted {
        operation: String,
        source: Option<StateId>,
        target: StateId,
    },

    /// A state transition completed successfully.
    TransitionCompleted {
        operation: String,
        source: Option<StateId>,
        target: StateId,
        duration: Option<Duration>,
    },

    /// A state transition failed.
    TransitionFailed {
        operation: String,
        source: Option<StateId>,
        target: Option<StateId>,
        retryable: bool,
    },

    /// A rollback operation started.
    RollbackStarted { from: StateId, to: StateId },

    /// A rollback operation completed.
    RollbackCompleted {
        from: StateId,
        to: StateId,
        duration: Option<Duration>,
    },

    /// Cleanup pass started.
    CleanupStarted { planned_states: usize },

    /// Cleanup pass completed.
    CleanupCompleted {
        removed_states: usize,
        space_freed: u64,
    },
}
