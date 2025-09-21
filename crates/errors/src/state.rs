//! State management error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum StateError {
    #[error("invalid state transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },

    #[error("state conflict: {message}")]
    Conflict { message: String },

    #[error("state not found: {id}")]
    StateNotFound { id: String },

    #[error("database error: {message}")]
    DatabaseError { message: String },

    #[error("transaction failed: {message}")]
    TransactionFailed { message: String },

    #[error("state corrupted: {message}")]
    StateCorrupted { message: String },

    #[error("rollback failed: {message}")]
    RollbackFailed { message: String },

    #[error("active state missing")]
    ActiveStateMissing,

    #[error("migration failed: {message}")]
    MigrationFailed { message: String },
}

impl UserFacingError for StateError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::Conflict { .. } => Some("Retry once the concurrent operation has completed."),
            Self::StateNotFound { .. } => Some("List available states with `sps2 history --all`."),
            Self::ActiveStateMissing => {
                Some("Run `sps2 check-health` to rebuild the active state.")
            }
            Self::MigrationFailed { .. } => {
                Some("Review the migration logs and rerun `sps2 check-health`.")
            }
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(self, Self::Conflict { .. } | Self::TransactionFailed { .. })
    }

    fn user_code(&self) -> Option<&'static str> {
        let code = match self {
            Self::InvalidTransition { .. } => "state.invalid_transition",
            Self::Conflict { .. } => "state.conflict",
            Self::StateNotFound { .. } => "state.state_not_found",
            Self::DatabaseError { .. } => "state.database_error",
            Self::TransactionFailed { .. } => "state.transaction_failed",
            Self::StateCorrupted { .. } => "state.state_corrupted",
            Self::RollbackFailed { .. } => "state.rollback_failed",
            Self::ActiveStateMissing => "state.active_state_missing",
            Self::MigrationFailed { .. } => "state.migration_failed",
        };
        Some(code)
    }
}
