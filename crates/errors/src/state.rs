//! State management error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
