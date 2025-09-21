use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::path::PathBuf;

/// Build system types supported by sps2
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildSystem {
    Autotools,
    CMake,
    Cargo,
    Make,
    Ninja,
    Custom,
}

/// Build phases for multi-stage operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildPhase {
    Source,
    Build,
    PostProcess,
    Package,
}

/// Identifier and configuration for a build session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSession {
    pub id: String,
    pub system: BuildSystem,
    pub cache_enabled: bool,
}

/// Target package for a build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildTarget {
    pub package: String,
    pub version: Version,
}

/// Descriptor for a command executed during a build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDescriptor {
    pub id: Option<String>,
    pub command: String,
    pub working_dir: PathBuf,
}

/// Stream for build log output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// Status updates for build phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PhaseStatus {
    Started,
    Completed {
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
}

/// Structured diagnostics emitted during a build session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuildDiagnostic {
    Warning {
        session_id: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    LogChunk {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        command_id: Option<String>,
        stream: LogStream,
        text: String,
    },
    CachePruned {
        removed_items: usize,
        freed_bytes: u64,
    },
}

/// Build-specific events consumed by the CLI and logging pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BuildEvent {
    /// Build session started with high-level context.
    Started {
        session: BuildSession,
        target: BuildTarget,
    },

    /// Build phase status update.
    PhaseStatus {
        session_id: String,
        phase: BuildPhase,
        status: PhaseStatus,
    },

    /// Build completed successfully.
    Completed {
        session_id: String,
        target: BuildTarget,
        artifacts: Vec<PathBuf>,
        duration_ms: u64,
    },

    /// Build failed during execution.
    Failed {
        session_id: String,
        target: BuildTarget,
        failure: super::FailureContext,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<BuildPhase>,
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<CommandDescriptor>,
    },

    /// Structured diagnostics for warning/log streaming.
    Diagnostic(BuildDiagnostic),
}
