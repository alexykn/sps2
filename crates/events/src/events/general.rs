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
    OperationStarted {
        operation: String,
    },
    
    /// Generic operation completion with success status
    OperationCompleted {
        operation: String,
        success: bool,
    },
    
    /// Generic operation failure with error details
    OperationFailed {
        operation: String,
        error: String,
    },
    
    /// User confirmation request for interactive operations
    UserConfirmationRequired {
        prompt: String,
        default: Option<bool>,
        timeout_seconds: Option<u64>,
    },
    
    /// User confirmation response received
    UserConfirmationReceived {
        response: bool,
    },
    
    /// System-level status notification
    SystemNotification {
        level: String,
        message: String,
        category: String,
    },
    
    /// Configuration validation results
    ConfigurationValidated {
        source: String,
        warnings: Vec<String>,
    },
    
    /// Configuration validation error
    ConfigurationError {
        field: String,
        error: String,
        suggested_fix: Option<String>,
    },
    
    /// Performance metric update
    PerformanceMetric {
        name: String,
        value: f64,
        unit: String,
        timestamp: Option<std::time::SystemTime>,
    },
    
    /// System resource usage update
    ResourceUsage {
        resource_type: String, // "memory", "disk", "network"
        used: u64,
        total: Option<u64>,
        unit: String,
    },
    
    /// Rate limiting applied to operation
    RateLimitApplied {
        operation: String,
        delay_ms: u64,
        reason: String,
    },
    
    /// Dependency conflict detected during resolution
    DependencyConflictDetected {
        conflicting_packages: Vec<String>,
        message: String,
    },
    
    /// Suggestions for resolving dependency conflicts
    DependencyConflictSuggestions {
        suggestions: Vec<String>,
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
    pub fn debug_with_context(message: impl Into<String>, context: HashMap<String, String>) -> Self {
        Self::DebugLog {
            message: message.into(),
            context,
        }
    }
}