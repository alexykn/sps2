#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Error types for the sps2 package manager
//!
//! This crate provides fine-grained error types organized by domain.
//! All error types implement Clone where possible for easier handling.

pub mod audit;
pub mod build;
pub mod config;
pub mod guard;
pub mod install;
pub mod network;
pub mod ops;
pub mod package;
pub mod platform;
pub mod state;
pub mod storage;
pub mod version;
pub mod signing;

// Re-export all error types at the root
pub use audit::AuditError;
pub use build::BuildError;
pub use config::ConfigError;
pub use guard::{
    DiscrepancyContext, DiscrepancySeverity, GuardError, GuardErrorSummary, RecommendedAction,
};
pub use install::InstallError;
pub use network::NetworkError;
pub use ops::OpsError;
pub use package::PackageError;
pub use platform::PlatformError;
pub use state::StateError;
pub use storage::StorageError;
pub use version::VersionError;
pub use signing::SigningError;

use thiserror::Error;

/// Generic error type for cross-crate boundaries
#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Error {
    #[error("network error: {0}")]
    Network(#[from] NetworkError),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("state error: {0}")]
    State(#[from] StateError),

    #[error("package error: {0}")]
    Package(#[from] PackageError),

    #[error("version error: {0}")]
    Version(#[from] VersionError),

    #[error("config error: {0}")]
    Config(#[from] ConfigError),

    #[error("build error: {0}")]
    Build(#[from] BuildError),

    #[error("audit error: {0}")]
    Audit(#[from] AuditError),

    #[error("install error: {0}")]
    Install(#[from] InstallError),

    #[error("ops error: {0}")]
    Ops(#[from] OpsError),

    #[error("guard error: {0}")]
    Guard(#[from] GuardError),

    #[error("platform error: {0}")]
    Platform(#[from] PlatformError),

    #[error("signing error: {0}")]
    Signing(#[from] SigningError),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("operation cancelled")]
    Cancelled,

    #[error("I/O error: {message}")]
    Io { message: String },
}

impl Error {
    /// Create an internal error with a message
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            message: err.to_string(),
        }
    }
}

impl From<semver::Error> for Error {
    fn from(err: semver::Error) -> Self {
        Self::Version(VersionError::ParseError {
            message: err.to_string(),
        })
    }
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        Self::State(StateError::DatabaseError {
            message: err.to_string(),
        })
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Internal(format!("JSON error: {err}"))
    }
}

impl From<minisign_verify::Error> for Error {
    fn from(err: minisign_verify::Error) -> Self {
        Self::Signing(SigningError::VerificationFailed { reason: err.to_string() })
    }
}

/// Result type alias for sps2 operations
pub type Result<T> = std::result::Result<T, Error>;
