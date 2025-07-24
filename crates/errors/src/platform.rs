//! Platform-specific operation errors

use crate::{BuildError, StorageError};
use thiserror::Error;

/// Errors that can occur during platform-specific operations
#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PlatformError {
    #[error("binary operation failed: {operation} on {binary_path} - {message}")]
    BinaryOperationFailed {
        operation: String,
        binary_path: String,
        message: String,
    },

    #[error("filesystem operation failed: {operation} - {message}")]
    FilesystemOperationFailed { operation: String, message: String },

    #[error("process execution failed: {command} - {message}")]
    ProcessExecutionFailed { command: String, message: String },

    #[error("platform capability not available: {capability}")]
    CapabilityUnavailable { capability: String },

    #[error("command not found: {command}")]
    CommandNotFound { command: String },

    #[error("invalid binary format: {path} - {message}")]
    InvalidBinaryFormat { path: String, message: String },

    #[error("signing operation failed: {binary_path} - {message}")]
    SigningFailed {
        binary_path: String,
        message: String,
    },

    #[error("permission denied: {operation} - {message}")]
    PermissionDenied { operation: String, message: String },
}

impl From<PlatformError> for BuildError {
    fn from(err: PlatformError) -> Self {
        match err {
            PlatformError::SigningFailed { message, .. } => BuildError::SigningError { message },
            PlatformError::BinaryOperationFailed {
                operation, message, ..
            } if operation.contains("sign") => BuildError::SigningError { message },
            PlatformError::ProcessExecutionFailed { command, message }
                if command.contains("git") =>
            {
                BuildError::Failed {
                    message: format!("git operation failed: {message}"),
                }
            }
            PlatformError::ProcessExecutionFailed { command, message }
                if command.contains("tar") || command.contains("zstd") =>
            {
                BuildError::ExtractionFailed { message }
            }
            PlatformError::FilesystemOperationFailed { message, .. } => BuildError::Failed {
                message: format!("filesystem operation failed: {message}"),
            },
            _ => BuildError::Failed {
                message: err.to_string(),
            },
        }
    }
}

impl From<PlatformError> for StorageError {
    fn from(err: PlatformError) -> Self {
        match err {
            PlatformError::FilesystemOperationFailed { operation, message } => {
                if operation.contains("clone") || operation.contains("apfs") {
                    StorageError::ApfsCloneFailed { message }
                } else if operation.contains("rename") || operation.contains("atomic") {
                    StorageError::AtomicRenameFailed { message }
                } else {
                    StorageError::IoError { message }
                }
            }
            PlatformError::PermissionDenied { message, .. } => {
                StorageError::PermissionDenied { path: message }
            }
            _ => StorageError::IoError {
                message: err.to_string(),
            },
        }
    }
}
