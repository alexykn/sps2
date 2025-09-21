use serde::{Deserialize, Serialize};
use sps2_types::StateId;

use super::FailureContext;

/// Context describing a state transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransitionContext {
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<StateId>,
    pub target: StateId,
}

/// Optional summary for completed transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Context for rollback operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackContext {
    pub from: StateId,
    pub to: StateId,
}

/// Optional summary for completed rollbacks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Summary for cleanup operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupSummary {
    pub planned_states: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_states: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_freed_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// State events emitted by state manager and install flows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StateEvent {
    TransitionStarted {
        context: StateTransitionContext,
    },
    TransitionCompleted {
        context: StateTransitionContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<TransitionSummary>,
    },
    TransitionFailed {
        context: StateTransitionContext,
        failure: FailureContext,
    },
    RollbackStarted {
        context: RollbackContext,
    },
    RollbackCompleted {
        context: RollbackContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<RollbackSummary>,
    },
    RollbackFailed {
        context: RollbackContext,
        failure: FailureContext,
    },
    CleanupStarted {
        summary: CleanupSummary,
    },
    CleanupCompleted {
        summary: CleanupSummary,
    },
    CleanupFailed {
        summary: CleanupSummary,
        failure: FailureContext,
    },
}
