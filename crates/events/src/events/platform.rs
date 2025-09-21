use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::FailureContext;

/// High-level category for a platform operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformOperationKind {
    Binary,
    Filesystem,
    Process,
    ToolDiscovery,
}

/// Descriptor for a process command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessCommandDescriptor {
    pub program: String,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
}

/// Context describing the operation being performed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformOperationContext {
    pub kind: PlatformOperationKind,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<ProcessCommandDescriptor>,
}

/// Optional metrics gathered for completed operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformOperationMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<Vec<String>>,
}

/// Platform events surfaced to consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformEvent {
    OperationStarted {
        context: PlatformOperationContext,
    },
    OperationCompleted {
        context: PlatformOperationContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        metrics: Option<PlatformOperationMetrics>,
    },
    OperationFailed {
        context: PlatformOperationContext,
        failure: FailureContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        metrics: Option<PlatformOperationMetrics>,
    },
}
