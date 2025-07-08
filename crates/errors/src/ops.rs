//! Operation orchestration error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OpsError {
    #[error("operation failed: {message}")]
    OperationFailed { message: String },

    #[error("component not found: {component}")]
    MissingComponent { component: String },

    #[error("invalid operation: {operation}")]
    InvalidOperation { operation: String },

    #[error("dependency error: {message}")]
    DependencyError { message: String },

    #[error("initialization failed: {message}")]
    InitializationFailed { message: String },

    #[error("command execution failed: {command}: {message}")]
    CommandFailed { command: String, message: String },

    #[error("health check failed: {component}: {message}")]
    HealthCheckFailed { component: String, message: String },

    #[error("context creation failed: {message}")]
    ContextCreationFailed { message: String },

    #[error("operation not supported: {operation}")]
    NotSupported { operation: String },

    #[error("concurrent operation limit exceeded")]
    ConcurrencyLimitExceeded,

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

    #[error("no current state")]
    NoCurrentState,

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

    #[error("event channel closed")]
    EventChannelClosed,
}
