use serde::{Deserialize, Serialize};
use sps2_types::StateId;
use std::time::Duration;

/// State management events for atomic operations and rollback
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StateEvent {
    /// State initialization started
    Initializing {
        state_id: StateId,
        operation: String,
        estimated_duration: Option<Duration>,
    },

    /// State created successfully
    Created {
        state_id: StateId,
        parent_id: Option<StateId>,
        operation: String,
    },

    /// State activation in progress
    Activating {
        state_id: StateId,
        from_state: Option<StateId>,
    },

    /// State activated successfully
    Activated {
        state_id: StateId,
        from_state: Option<StateId>,
    },

    /// State transition preparation
    TransitionPreparing {
        from: StateId,
        to: StateId,
        operation: String,
        packages_affected: usize,
    },

    /// State transition validation
    TransitionValidating {
        from: StateId,
        to: StateId,
        validation_checks: usize,
    },

    /// State transition validation complete
    TransitionValidationComplete {
        from: StateId,
        to: StateId,
        checks_passed: usize,
        warnings: usize,
    },

    /// State transition executing
    TransitionExecuting {
        from: StateId,
        to: StateId,
        operation: String,
    },

    /// State transition completed successfully
    TransitionCompleted {
        from: StateId,
        to: StateId,
        operation: String,
        duration: Duration,
    },

    /// State transition failed
    TransitionFailed {
        from: StateId,
        to: StateId,
        operation: String,
        error: String,
        rollback_available: bool,
    },

    /// Rollback initiated
    RollbackInitiated {
        from: StateId,
        to: StateId,
        reason: String,
        automatic: bool,
    },

    /// Rollback validation in progress
    RollbackValidating {
        target_state: StateId,
        safety_checks: usize,
    },

    /// Rollback executing
    RollbackExecuting {
        from: StateId,
        to: StateId,
        packages_affected: usize,
    },

    /// Rollback completed successfully
    RollbackCompleted {
        from: StateId,
        to: StateId,
        duration: Duration,
        packages_reverted: usize,
    },

    /// Rollback failed
    RollbackFailed {
        from: StateId,
        to: StateId,
        error: String,
        recovery_options: Vec<String>,
    },

    /// State cleanup started
    CleanupStarted {
        states_to_remove: usize,
        estimated_space_freed: u64,
    },

    /// State cleanup progress
    CleanupProgress {
        states_processed: usize,
        total_states: usize,
        space_freed: u64,
    },

    /// State cleanup completed
    CleanupCompleted {
        states_removed: usize,
        space_freed: u64,
        duration: Duration,
    },

    /// Two-phase commit started
    TwoPhaseCommitStarting {
        state_id: StateId,
        parent_state_id: StateId,
        operation: String,
    },

    /// Two-phase commit phase one started
    TwoPhaseCommitPhaseOneStarting {
        state_id: StateId,
        operation: String,
    },

    /// Two-phase commit phase one completed
    TwoPhaseCommitPhaseOneCompleted {
        state_id: StateId,
        operation: String,
    },

    /// Two-phase commit phase two started
    TwoPhaseCommitPhaseTwoStarting {
        state_id: StateId,
        operation: String,
    },

    /// Two-phase commit phase two completed
    TwoPhaseCommitPhaseTwoCompleted {
        state_id: StateId,
        operation: String,
    },

    /// Two-phase commit completed
    TwoPhaseCommitCompleted {
        state_id: StateId,
        parent_state_id: StateId,
        operation: String,
    },

    /// Two-phase commit failed
    TwoPhaseCommitFailed {
        state_id: StateId,
        operation: String,
        error: String,
        phase: String,
    },
}
