//! Storage and filesystem-related error types

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
