#![warn(mismatched_lifetime_syntaxes)]
#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Error types for the sps2 package manager
//!
//! This crate provides fine-grained error types organized by domain.
//! All error types implement Clone where possible for easier handling.

use std::borrow::Cow;

use thiserror::Error;

pub mod audit;
pub mod build;
pub mod config;
pub mod guard;
pub mod install;
pub mod network;
pub mod ops;
pub mod package;
pub mod platform;
pub mod signing;
pub mod state;
pub mod storage;
pub mod version;

// Re-export all error types at the root
pub use audit::AuditError;
pub use build::BuildError;
pub use config::ConfigError;
pub use guard::{DiscrepancySeverity, GuardError};
pub use install::InstallError;
pub use network::NetworkError;
pub use ops::OpsError;
pub use package::PackageError;
pub use platform::PlatformError;
pub use signing::SigningError;
pub use state::StateError;
pub use storage::StorageError;
pub use version::VersionError;

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
    Io {
        #[cfg_attr(feature = "serde", serde(with = "io_kind_as_str"))]
        kind: std::io::ErrorKind,
        message: String,
        #[cfg_attr(feature = "serde", serde(with = "opt_path_buf"))]
        path: Option<std::path::PathBuf>,
    },
}

impl Error {
    /// Create an internal error with a message
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Create an Io error with an associated path
    pub fn io_with_path(err: &std::io::Error, path: impl Into<std::path::PathBuf>) -> Self {
        Self::Io {
            kind: err.kind(),
            message: err.to_string(),
            path: Some(path.into()),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            kind: err.kind(),
            message: err.to_string(),
            path: None,
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
        Self::Signing(SigningError::VerificationFailed {
            reason: err.to_string(),
        })
    }
}

/// Result type alias for sps2 operations
pub type Result<T> = std::result::Result<T, Error>;

/// Minimal interface for rendering user-facing error information without
/// requiring heavyweight envelopes.
pub trait UserFacingError {
    /// Short message suitable for CLI output.
    fn user_message(&self) -> Cow<'_, str>;

    /// Optional remediation hint.
    fn user_hint(&self) -> Option<&'static str> {
        None
    }

    /// Whether retrying the same operation is likely to succeed.
    fn is_retryable(&self) -> bool {
        false
    }

    /// Stable error code for analytics / structured reporting.
    fn user_code(&self) -> Option<&'static str> {
        None
    }
}

impl UserFacingError for Error {
    fn user_message(&self) -> Cow<'_, str> {
        match self {
            Error::Network(err) => err.user_message(),
            Error::Install(err) => err.user_message(),
            Error::Ops(err) => err.user_message(),
            Error::Io { message, .. } => Cow::Owned(message.clone()),
            _ => Cow::Owned(self.to_string()),
        }
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Error::Network(err) => err.user_hint(),
            Error::Install(err) => err.user_hint(),
            Error::Ops(err) => err.user_hint(),
            Error::Config(_) => Some("Check your sps2 configuration file."),
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            Error::Network(err) => err.is_retryable(),
            Error::Install(err) => err.is_retryable(),
            Error::Ops(err) => err.is_retryable(),
            Error::Io { .. } => true,
            _ => false,
        }
    }

    fn user_code(&self) -> Option<&'static str> {
        match self {
            Error::Network(err) => err.user_code(),
            Error::Storage(err) => err.user_code(),
            Error::State(err) => err.user_code(),
            Error::Package(err) => err.user_code(),
            Error::Version(err) => err.user_code(),
            Error::Config(err) => err.user_code(),
            Error::Build(err) => err.user_code(),
            Error::Audit(err) => err.user_code(),
            Error::Install(err) => err.user_code(),
            Error::Ops(err) => err.user_code(),
            Error::Guard(err) => err.user_code(),
            Error::Platform(err) => err.user_code(),
            Error::Signing(err) => err.user_code(),
            Error::Internal(_) => Some("error.internal"),
            Error::Cancelled => Some("error.cancelled"),
            Error::Io { .. } => Some("error.io"),
        }
    }
}

// Serde helper modules for optional path and io::ErrorKind as string
#[cfg(feature = "serde")]
mod io_kind_as_str {
    use serde::{Deserialize, Deserializer, Serializer};
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn serialize<S>(kind: &std::io::ErrorKind, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_str(&format!("{kind:?}"))
    }
    pub fn deserialize<'de, D>(deserializer: D) -> Result<std::io::ErrorKind, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // Best effort mapping; default to Other
        Ok(match s.as_str() {
            "NotFound" => std::io::ErrorKind::NotFound,
            "PermissionDenied" => std::io::ErrorKind::PermissionDenied,
            "ConnectionRefused" => std::io::ErrorKind::ConnectionRefused,
            "ConnectionReset" => std::io::ErrorKind::ConnectionReset,
            "ConnectionAborted" => std::io::ErrorKind::ConnectionAborted,
            "NotConnected" => std::io::ErrorKind::NotConnected,
            "AddrInUse" => std::io::ErrorKind::AddrInUse,
            "AddrNotAvailable" => std::io::ErrorKind::AddrNotAvailable,
            "BrokenPipe" => std::io::ErrorKind::BrokenPipe,
            "AlreadyExists" => std::io::ErrorKind::AlreadyExists,
            "WouldBlock" => std::io::ErrorKind::WouldBlock,
            "InvalidInput" => std::io::ErrorKind::InvalidInput,
            "InvalidData" => std::io::ErrorKind::InvalidData,
            "TimedOut" => std::io::ErrorKind::TimedOut,
            "WriteZero" => std::io::ErrorKind::WriteZero,
            "Interrupted" => std::io::ErrorKind::Interrupted,
            "Unsupported" => std::io::ErrorKind::Unsupported,
            "UnexpectedEof" => std::io::ErrorKind::UnexpectedEof,
            _ => std::io::ErrorKind::Other,
        })
    }
}

#[cfg(feature = "serde")]
mod opt_path_buf {
    use serde::{Deserialize, Deserializer, Serializer};
    #[allow(clippy::ref_option)]
    pub fn serialize<S>(path: &Option<std::path::PathBuf>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match path {
            Some(pb) => s.serialize_some(&pb.display().to_string()),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<std::path::PathBuf>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt = Option::<String>::deserialize(deserializer)?;
        Ok(opt.map(std::path::PathBuf::from))
    }
}
