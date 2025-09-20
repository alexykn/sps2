use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::path::PathBuf;
use std::time::Duration;

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

/// Build-specific events consumed by the CLI and logging pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BuildEvent {
    /// Build session started with comprehensive context
    SessionStarted {
        session_id: String,
        package: String,
        version: Version,
        build_system: BuildSystem,
        cache_enabled: bool,
    },

    /// Build phase started
    PhaseStarted {
        session_id: String,
        package: String,
        phase: BuildPhase,
        estimated_duration: Option<Duration>,
    },

    /// Build phase completed
    PhaseCompleted {
        session_id: String,
        package: String,
        phase: BuildPhase,
        duration: Duration,
    },

    /// Build command started
    CommandStarted {
        session_id: String,
        package: String,
        command_id: String,
        build_system: BuildSystem,
        command: String,
        working_dir: PathBuf,
        timeout: Option<Duration>,
    },

    /// Real-time build output
    StepOutput {
        session_id: String,
        package: String,
        command_id: String,
        line: String,
        is_stderr: bool,
    },

    /// Build completed successfully
    Completed {
        session_id: String,
        package: String,
        version: Version,
        path: PathBuf,
        duration: Duration,
    },

    /// Build failed
    Failed {
        session_id: String,
        package: String,
        version: Version,
        error: String,
        phase: Option<BuildPhase>,
        recovery_suggestions: Vec<String>,
    },

    /// Build warning encountered
    Warning {
        session_id: String,
        package: String,
        message: String,
        source: Option<String>,
    },

    /// Build cache cleaned
    CacheCleaned {
        removed_items: usize,
        freed_bytes: u64,
    },

    /// Build cleaned up
    Cleaned { session_id: String, package: String },

    /// Build resource usage update
    ResourceUsage {
        session_id: String,
        package: String,
        cpu_percent: f64,
        memory_mb: u64,
        disk_usage_mb: u64,
    },

    /// Build orchestration phase started (recipe parsing, env setup, etc.)
    OrchestrationPhaseStarted {
        phase: String,
        description: Option<String>,
    },

    /// Build orchestration phase completed
    OrchestrationPhaseCompleted {
        phase: String,
        success: bool,
        duration: Option<Duration>,
    },
}
