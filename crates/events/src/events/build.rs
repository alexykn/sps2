use serde::{Deserialize, Serialize};
use sps2_types::Version;
use std::path::PathBuf;
use std::time::Duration;

/// Build system types supported by sps2
#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub enum BuildPhase {
    Source,
    Build,
    PostProcess,
    Package,
}

/// Build cache strategies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CacheStrategy {
    Skip,
    Populate,
    Use,
}

/// Build isolation levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IsolationLevel {
    None,
    Network,
    Full,
}

/// Build-specific events for the event system
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

    /// Build session completed
    SessionCompleted {
        session_id: String,
        package: String,
        version: Version,
        duration: Duration,
        artifacts_created: usize,
        cache_populated: bool,
    },

    /// Build queued awaiting dependencies
    Queued {
        session_id: String,
        package: String,
        version: Version,
        position_in_queue: usize,
        build_dependencies: Vec<String>,
    },

    /// Build phase started
    PhaseStarted {
        session_id: String,
        package: String,
        phase: BuildPhase,
        estimated_duration: Option<Duration>,
    },

    /// Build phase progress update
    PhaseProgress {
        session_id: String,
        package: String,
        phase: BuildPhase,
        current_step: usize,
        total_steps: usize,
        current_step_name: String,
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

    /// Build command completed
    CommandCompleted {
        session_id: String,
        package: String,
        command_id: String,
        exit_code: i32,
        duration: Duration,
    },

    /// Build dependency resolution started
    DependencyResolutionStarted {
        session_id: String,
        package: String,
        build_deps_count: usize,
    },

    /// Build dependency installed
    DependencyInstalled {
        session_id: String,
        package: String,
        dependency: String,
        dependency_version: Version,
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

    /// Build retrying after failure
    Retrying {
        session_id: String,
        package: String,
        attempt: usize,
        max_attempts: usize,
        reason: String,
    },

    /// Build warning encountered
    Warning {
        session_id: String,
        package: String,
        message: String,
        source: Option<String>,
    },

    /// Build environment configured
    EnvironmentConfigured {
        session_id: String,
        package: String,
        isolation_level: IsolationLevel,
        network_enabled: bool,
        env_vars_count: usize,
    },

    /// Build cache strategy determined
    CacheStrategy {
        session_id: String,
        package: String,
        strategy: CacheStrategy,
        cache_key: String,
    },

    /// Build cache hit
    CacheHit {
        cache_key: String,
        artifacts_count: usize,
    },

    /// Build cache miss
    CacheMiss { cache_key: String, reason: String },

    /// Build cache updated
    CacheUpdated {
        cache_key: String,
        artifacts_count: usize,
    },

    /// Build cache cleaned
    CacheCleaned {
        removed_items: usize,
        freed_bytes: u64,
    },

    /// Build checkpoint created
    CheckpointCreated {
        session_id: String,
        package: String,
        checkpoint_id: String,
        stage: String,
    },

    /// Build checkpoint restored
    CheckpointRestored {
        session_id: String,
        package: String,
        checkpoint_id: String,
        stage: String,
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
}
