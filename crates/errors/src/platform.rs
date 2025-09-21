//! Platform-specific operation errors

use std::borrow::Cow;

use crate::{BuildError, StorageError, UserFacingError};
use thiserror::Error;

/// Errors that can occur during platform-specific operations
#[derive(Debug, Clone, Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
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

    #[error("tool '{tool}' not found. {suggestion}")]
    ToolNotFound {
        tool: String,
        suggestion: String,
        searched_paths: Vec<std::path::PathBuf>,
    },

    #[error("multiple tools not found: {}", .tools.join(", "))]
    MultipleToolsNotFound {
        tools: Vec<String>,
        suggestions: Vec<String>,
    },

    #[error("command failed: {command} - {error}")]
    CommandFailed { command: String, error: String },

    #[error("configuration error: {message}")]
    ConfigError { message: String },
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

impl UserFacingError for PlatformError {
    fn user_message(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }

    fn user_hint(&self) -> Option<&'static str> {
        match self {
            Self::CommandNotFound { .. }
            | Self::ToolNotFound { .. }
            | Self::MultipleToolsNotFound { .. } => {
                Some("Install the required tool or adjust your PATH, then retry.")
            }
            Self::CapabilityUnavailable { .. } => {
                Some("Enable the required platform capability or use an alternative workflow.")
            }
            Self::PermissionDenied { .. } => {
                Some("Adjust filesystem permissions or rerun the command with elevated privileges.")
            }
            _ => None,
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::BinaryOperationFailed { .. }
                | Self::FilesystemOperationFailed { .. }
                | Self::ProcessExecutionFailed { .. }
                | Self::CommandFailed { .. }
                | Self::CommandNotFound { .. }
                | Self::ToolNotFound { .. }
                | Self::MultipleToolsNotFound { .. }
                | Self::PermissionDenied { .. }
        )
    }

    fn user_code(&self) -> Option<&'static str> {
        let code = match self {
            Self::BinaryOperationFailed { .. } => "platform.binary_operation_failed",
            Self::FilesystemOperationFailed { .. } => "platform.filesystem_operation_failed",
            Self::ProcessExecutionFailed { .. } => "platform.process_execution_failed",
            Self::CapabilityUnavailable { .. } => "platform.capability_unavailable",
            Self::CommandNotFound { .. } => "platform.command_not_found",
            Self::InvalidBinaryFormat { .. } => "platform.invalid_binary_format",
            Self::SigningFailed { .. } => "platform.signing_failed",
            Self::PermissionDenied { .. } => "platform.permission_denied",
            Self::ToolNotFound { .. } => "platform.tool_not_found",
            Self::MultipleToolsNotFound { .. } => "platform.multiple_tools_not_found",
            Self::CommandFailed { .. } => "platform.command_failed",
            Self::ConfigError { .. } => "platform.config_error",
        };
        Some(code)
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
