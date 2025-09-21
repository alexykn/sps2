//! Storage and filesystem-related error types

use std::borrow::Cow;

use crate::UserFacingError;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum StorageError {
    #[error("disk full: {path}")]
    DiskFull { path: String },

    #[error("permission denied: {path}")]
    PermissionDenied { path: String },

    #[error("path not found: {path}")]
    PathNotFound { path: String },

    #[error("directory not found: {path}")]
    DirectoryNotFound { path: std::path::PathBuf },

    #[error("already exists: {path}")]
    AlreadyExists { path: String },

    #[error("IO error: {message}")]
    IoError { message: String },

    #[error("corrupted data: {message}")]
    CorruptedData { message: String },

    #[error("invalid path: {path}")]
    InvalidPath { path: String },

    #[error("lock acquisition failed: {path}")]
    LockFailed { path: String },

    #[error("APFS clone failed: {message}")]
    ApfsCloneFailed { message: String },

    #[error("atomic rename failed: {message}")]
    AtomicRenameFailed { message: String },

    #[error("package not found: {hash}")]
    PackageNotFound { hash: String },
}

impl From<std::io::Error> for StorageError {
    fn from(err: std::io::Error) -> Self {
        // Without a known path, avoid inventing placeholders; preserve message only
        Self::IoError {
            message: err.to_string(),
        }
    }
}

impl StorageError {
    /// Convert an `io::Error` into a `StorageError` with an associated path
    #[must_use]
    pub fn from_io_with_path(err: &std::io::Error, path: &std::path::Path) -> Self {
        match err.kind() {
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied {
                path: path.display().to_string(),
            },
            std::io::ErrorKind::NotFound => Self::PathNotFound {
                path: path.display().to_string(),
            },
            std::io::ErrorKind::AlreadyExists => Self::AlreadyExists {
                path: path.display().to_string(),
            },
            _ => Self::IoError {
                message: format!("{}: {}", path.display(), err),
            },
        }
    }
}

impl UserFacingError for StorageError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::DiskFull { .. } => Some("Free up disk space under /opt/pm and retry."),
            Self::PermissionDenied { .. } => {
                Some("Adjust filesystem permissions or retry with elevated privileges.")
            }
            Self::LockFailed { .. } => {
                Some("Wait for other package-manager operations to finish, then retry.")
            }
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(self, Self::LockFailed { .. } | Self::IoError { .. })
    }

    fn user_code(&self) -> Option<&'static str> {
        let code = match self {
            Self::DiskFull { .. } => "storage.disk_full",
            Self::PermissionDenied { .. } => "storage.permission_denied",
            Self::PathNotFound { .. } => "storage.path_not_found",
            Self::DirectoryNotFound { .. } => "storage.directory_not_found",
            Self::AlreadyExists { .. } => "storage.already_exists",
            Self::IoError { .. } => "storage.io_error",
            Self::CorruptedData { .. } => "storage.corrupted_data",
            Self::InvalidPath { .. } => "storage.invalid_path",
            Self::LockFailed { .. } => "storage.lock_failed",
            Self::ApfsCloneFailed { .. } => "storage.apfs_clone_failed",
            Self::AtomicRenameFailed { .. } => "storage.atomic_rename_failed",
            Self::PackageNotFound { .. } => "storage.package_not_found",
        };
        Some(code)
    }
}
