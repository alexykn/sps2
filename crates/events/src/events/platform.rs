//! Platform-specific operation events

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Platform operation events for tracking platform-specific operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum PlatformEvent {
    /// Binary operation started (`install_name_tool`, `otool`, `codesign`)
    BinaryOperationStarted {
        /// Operation name (e.g., `get_install_name`, `sign_binary`)
        operation: String,
        /// Path to the binary being operated on
        binary_path: String,
        /// Additional context for the operation
        context: HashMap<String, String>,
    },

    /// Binary operation completed successfully
    BinaryOperationCompleted {
        /// Operation name that completed
        operation: String,
        /// Path to the binary that was operated on
        binary_path: String,
        /// List of changes made during the operation
        changes_made: Vec<String>,
        /// Duration of the operation in milliseconds
        duration_ms: u64,
    },

    /// Binary operation failed
    BinaryOperationFailed {
        /// Operation name that failed
        operation: String,
        /// Path to the binary that was being operated on
        binary_path: String,
        /// Error message
        error_message: String,
        /// Duration before failure in milliseconds
        duration_ms: u64,
    },

    /// Filesystem operation started (APFS clone, atomic operations)
    FilesystemOperationStarted {
        /// Operation name (e.g., `clone_file`, `atomic_rename`)
        operation: String,
        /// Source path (if applicable)
        source_path: Option<String>,
        /// Target path
        target_path: String,
        /// Additional operation context
        context: HashMap<String, String>,
    },

    /// Filesystem operation completed successfully
    FilesystemOperationCompleted {
        /// Operation name that completed
        operation: String,
        /// List of paths affected by the operation
        paths_affected: Vec<String>,
        /// Duration of the operation in milliseconds
        duration_ms: u64,
    },

    /// Filesystem operation failed
    FilesystemOperationFailed {
        /// Operation name that failed
        operation: String,
        /// Paths involved in the failed operation
        paths_involved: Vec<String>,
        /// Error message
        error_message: String,
        /// Duration before failure in milliseconds
        duration_ms: u64,
    },

    /// Process execution started
    ProcessExecutionStarted {
        /// Command being executed
        command: String,
        /// Command arguments
        args: Vec<String>,
        /// Working directory (if set)
        working_dir: Option<String>,
    },

    /// Process execution completed
    ProcessExecutionCompleted {
        /// Command that was executed
        command: String,
        /// Exit code from the process
        exit_code: i32,
        /// Duration of execution in milliseconds
        duration_ms: u64,
        /// Size of stdout in bytes
        stdout_bytes: usize,
        /// Size of stderr in bytes
        stderr_bytes: usize,
    },

    /// Process execution failed
    ProcessExecutionFailed {
        /// Command that failed
        command: String,
        /// Error message
        error_message: String,
        /// Duration before failure in milliseconds
        duration_ms: u64,
    },

    /// Platform capability check started
    CapabilityCheckStarted {
        /// Capability being checked (e.g., `codesign_available`)
        capability: String,
    },

    /// Platform capability check completed
    CapabilityCheckCompleted {
        /// Capability that was checked
        capability: String,
        /// Whether the capability is available
        available: bool,
        /// Additional details about the capability
        details: HashMap<String, String>,
    },
}