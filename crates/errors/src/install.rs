//! Installation system error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

const HINT_WAIT_AND_RETRY: &str = "Wait for pending operations to finish, then retry.";
const HINT_RETRY_LATER: &str = "Retry the operation; the service may recover shortly.";
const HINT_DOWNLOAD_TIMEOUT: &str =
    "Retry the download or increase the timeout with --download-timeout.";

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum InstallError {
    #[error("package not found: {package}")]
    PackageNotFound { package: String },

    #[error("extraction failed: {message}")]
    ExtractionFailed { message: String },

    #[error("atomic operation failed: {message}")]
    AtomicOperationFailed { message: String },

    #[error("filesystem operation failed: {operation} on {path}: {message}")]
    FilesystemError {
        operation: String,
        path: String,
        message: String,
    },

    #[error("state not found: {state_id}")]
    StateNotFound { state_id: String },

    #[error("package has dependents: {package}")]
    PackageHasDependents { package: String },

    #[error("no packages specified")]
    NoPackagesSpecified,

    #[error("local package not found: {path}")]
    LocalPackageNotFound { path: String },

    #[error("invalid package file {path}: {message}")]
    InvalidPackageFile { path: String, message: String },

    #[error("task execution failed: {message}")]
    TaskError { message: String },

    #[error("package not installed: {package}")]
    PackageNotInstalled { package: String },

    #[error("concurrency error: {message}")]
    ConcurrencyError { message: String },

    #[error("download timeout: {package} from {url} after {timeout_seconds}s")]
    DownloadTimeout {
        package: String,
        url: String,
        timeout_seconds: u64,
    },

    #[error("missing download URL for package: {package}")]
    MissingDownloadUrl { package: String },

    #[error("missing local path for package: {package}")]
    MissingLocalPath { package: String },

    #[error("temporary file error: {message}")]
    TempFileError { message: String },

    #[error("operation timeout: {message}")]
    OperationTimeout { message: String },

    #[error("no progress detected: {message}")]
    NoProgress { message: String },
}

impl UserFacingError for InstallError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::ConcurrencyError { .. } => Some(HINT_WAIT_AND_RETRY),
            Self::OperationTimeout { .. } | Self::NoProgress { .. } => Some(HINT_RETRY_LATER),
            Self::DownloadTimeout { .. } => Some(HINT_DOWNLOAD_TIMEOUT),
            Self::MissingDownloadUrl { .. } | Self::MissingLocalPath { .. } => {
                Some("Ensure the package manifest includes a valid source.")
            }
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ConcurrencyError { .. }
                | Self::OperationTimeout { .. }
                | Self::NoProgress { .. }
                | Self::DownloadTimeout { .. }
        )
    }
}
