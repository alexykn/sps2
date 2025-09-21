use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// General utility events for warnings, errors, and operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GeneralEvent {
    /// Generic warning message with optional context
    Warning {
        message: String,
        context: Option<String>,
    },

    /// Generic error message with optional details
    Error {
        message: String,
        details: Option<String>,
    },

    /// Debug logging with structured context
    DebugLog {
        message: String,
        context: HashMap<String, String>,
    },

    /// Generic operation started notification
    OperationStarted { operation: String },

    /// Generic operation completion with success status
    OperationCompleted { operation: String, success: bool },

    /// Generic operation failure with error details
    OperationFailed {
        operation: String,
        failure: super::FailureContext,
    },

    /// Check mode preview of planned action
    CheckModePreview {
        operation: String,
        action: String,
        details: std::collections::HashMap<String, String>,
    },

    /// Check mode summary of all planned changes
    CheckModeSummary {
        operation: String,
        total_changes: usize,
        categories: std::collections::HashMap<String, usize>,
    },
}

impl GeneralEvent {
    /// Create a warning event
    pub fn warning(message: impl Into<String>) -> Self {
        Self::Warning {
            message: message.into(),
            context: None,
        }
    }

    /// Create a warning event with context
    pub fn warning_with_context(message: impl Into<String>, context: impl Into<String>) -> Self {
        Self::Warning {
            message: message.into(),
            context: Some(context.into()),
        }
    }

    /// Create an error event
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            details: None,
        }
    }

    /// Create an error event with details
    pub fn error_with_details(message: impl Into<String>, details: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            details: Some(details.into()),
        }
    }

    /// Create a debug log event
    pub fn debug(message: impl Into<String>) -> Self {
        Self::DebugLog {
            message: message.into(),
            context: HashMap::new(),
        }
    }

    /// Create a debug log event with context
    pub fn debug_with_context(
        message: impl Into<String>,
        context: HashMap<String, String>,
    ) -> Self {
        Self::DebugLog {
            message: message.into(),
            context,
        }
    }

    /// Create an operation failed event with structured error fields
    pub fn operation_failed(operation: impl Into<String>, failure: super::FailureContext) -> Self {
        Self::OperationFailed {
            operation: operation.into(),
            failure,
        }
    }
}
