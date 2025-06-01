//! Installation system error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum InstallError {
    #[error("installation failed: {message}")]
    Failed { message: String },

    #[error("dependency resolution failed: {message}")]
    ResolutionFailed { message: String },

    #[error("package not found: {package}")]
    PackageNotFound { package: String },

    #[error("version conflict: {message}")]
    VersionConflict { message: String },

    #[error("circular dependency detected: {packages}")]
    CircularDependency { packages: String },

    #[error("download failed: {url}")]
    DownloadFailed { url: String },

    #[error("verification failed: {package}")]
    VerificationFailed { package: String },

    #[error("extraction failed: {message}")]
    ExtractionFailed { message: String },

    #[error("atomic operation failed: {message}")]
    AtomicOperationFailed { message: String },

    #[error("rollback failed: {message}")]
    RollbackFailed { message: String },

    #[error("insufficient disk space: {required} bytes required")]
    InsufficientSpace { required: u64 },

    #[error("installation cancelled by user")]
    Cancelled,

    #[error("parallel installation failed: {message}")]
    ParallelFailed { message: String },

    #[error("state transition failed: {message}")]
    StateTransitionFailed { message: String },

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
