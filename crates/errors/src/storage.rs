//! Storage and filesystem-related error types

use thiserror::Error;

#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
}

impl From<std::io::Error> for StorageError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied {
                path: String::from("<unknown>"),
            },
            std::io::ErrorKind::NotFound => Self::PathNotFound {
                path: String::from("<unknown>"),
            },
            std::io::ErrorKind::AlreadyExists => Self::AlreadyExists {
                path: String::from("<unknown>"),
            },
            _ => Self::IoError {
                message: err.to_string(),
            },
        }
    }
}
