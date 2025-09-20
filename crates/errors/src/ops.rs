//! Operation orchestration error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

const HINT_PROVIDE_PACKAGE: &str =
    "Provide at least one package spec (e.g. `sps2 install ripgrep`).";

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum OpsError {
    #[error("operation failed: {message}")]
    OperationFailed { message: String },

    #[error("component not found: {component}")]
    MissingComponent { component: String },

    #[error("invalid operation: {operation}")]
    InvalidOperation { operation: String },

    #[error("serialization error: {message}")]
    SerializationError { message: String },

    #[error("no packages specified")]
    NoPackagesSpecified,

    #[error("recipe not found: {path}")]
    RecipeNotFound { path: String },

    #[error("invalid recipe {path}: {reason}")]
    InvalidRecipe { path: String, reason: String },

    #[error("package not found: {package}")]
    PackageNotFound { package: String },

    #[error("no previous state")]
    NoPreviousState,

    #[error("state not found: {state_id}")]
    StateNotFound { state_id: uuid::Uuid },

    #[error("repository sync failed: {message}")]
    RepoSyncFailed { message: String },

    #[error("self-update failed: {message}")]
    SelfUpdateFailed { message: String },

    #[error("state verification failed: {discrepancies} discrepancies found in state {state_id}")]
    VerificationFailed {
        discrepancies: usize,
        state_id: String,
    },

    #[error("staging directory not found: {path} (for package {package})")]
    StagingDirectoryNotFound { path: String, package: String },

    #[error("invalid staging directory {path}: {reason}")]
    InvalidStagingDirectory { path: String, reason: String },
}

impl UserFacingError for OpsError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::NoPackagesSpecified => Some(HINT_PROVIDE_PACKAGE),
            Self::NoPreviousState => Some("Create a state snapshot before attempting rollback."),
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(self, Self::NoPackagesSpecified | Self::NoPreviousState)
    }
}
