//! Core types and utilities for build systems

use crate::BuildEnvironment;
use sps2_errors::Error;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Build system context containing all necessary information for building
pub struct BuildSystemContext {
    /// Build environment
    pub env: BuildEnvironment,
    /// Source directory
    pub source_dir: PathBuf,
    /// Build directory (may be same as source for in-source builds)
    pub build_dir: PathBuf,
    /// Installation prefix
    pub prefix: PathBuf,
    /// Number of parallel jobs
    pub jobs: usize,
    /// Additional environment variables
    pub extra_env: Arc<RwLock<HashMap<String, String>>>,
    /// Whether network access is allowed
    pub network_allowed: bool,
    /// Cache configuration
    pub cache_config: Option<CacheConfig>,
}

impl BuildSystemContext {
    /// Create a new build context
    pub fn new(env: BuildEnvironment, source_dir: PathBuf) -> Self {
        let prefix = PathBuf::from("/opt/pm/live");
        let jobs = env
            .env_vars()
            .get("JOBS")
            .and_then(|j| j.parse().ok())
            .unwrap_or(1);

        Self {
            env,
            build_dir: source_dir.clone(),
            source_dir,
            prefix,
            jobs,
            extra_env: Arc::new(RwLock::new(HashMap::new())),
            network_allowed: false,
            cache_config: None,
        }
    }

    /// Set build directory for out-of-source builds
    #[must_use]
    pub fn with_build_dir(mut self, build_dir: PathBuf) -> Self {
        self.build_dir = build_dir;
        self
    }

    /// Add extra environment variables
    #[must_use]
    pub fn with_extra_env(mut self, env: HashMap<String, String>) -> Self {
        self.extra_env = Arc::new(RwLock::new(env));
        self
    }

    /// Set network access permission
    #[must_use]
    pub fn with_network_allowed(mut self, allowed: bool) -> Self {
        self.network_allowed = allowed;
        self
    }

    /// Set cache configuration
    #[must_use]
    pub fn with_cache_config(mut self, config: CacheConfig) -> Self {
        self.cache_config = Some(config);
        self
    }

    /// Get all environment variables for the build
    pub fn get_all_env_vars(&self) -> HashMap<String, String> {
        let mut vars = self.env.env_vars().clone();
        if let Ok(extra) = self.extra_env.read() {
            vars.extend(extra.clone());
        }

        vars
    }

    /// Execute a command in the build context
    ///
    /// # Errors
    ///
    /// Returns an error if command execution fails
    pub async fn execute(
        &self,
        program: &str,
        args: &[&str],
        working_dir: Option<&std::path::Path>,
    ) -> Result<crate::BuildCommandResult, Error> {
        self.env.execute_command(program, args, working_dir).await
    }
}

impl Clone for BuildSystemContext {
    fn clone(&self) -> Self {
        Self {
            env: self.env.clone(),
            source_dir: self.source_dir.clone(),
            build_dir: self.build_dir.clone(),
            prefix: self.prefix.clone(),
            jobs: self.jobs,
            extra_env: Arc::clone(&self.extra_env),
            network_allowed: self.network_allowed,
            cache_config: self.cache_config.clone(),
        }
    }
}

impl std::fmt::Debug for BuildSystemContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildSystemContext")
            .field("env", &self.env)
            .field("source_dir", &self.source_dir)
            .field("build_dir", &self.build_dir)
            .field("prefix", &self.prefix)
            .field("jobs", &self.jobs)
            .field("extra_env", &self.extra_env)
            .field("network_allowed", &self.network_allowed)
            .field("cache_config", &self.cache_config)
            .finish()
    }
}

/// Build system configuration
#[derive(Clone, Debug, Default)]
pub struct BuildSystemConfig {
    /// Whether out-of-source builds are supported
    pub supports_out_of_source: bool,
    /// Whether parallel builds are supported
    pub supports_parallel_builds: bool,
    /// Whether incremental builds are supported
    pub supports_incremental_builds: bool,
    /// Default configure arguments
    pub default_configure_args: Vec<String>,
    /// Default build arguments
    pub default_build_args: Vec<String>,
    /// Environment variable prefix for options
    pub env_prefix: Option<String>,
    /// File patterns to watch for changes
    pub watch_patterns: Vec<String>,
}

/// Test results from running the test suite
#[derive(Clone, Debug)]
pub struct TestResults {
    /// Total number of tests
    pub total: usize,
    /// Number of passed tests
    pub passed: usize,
    /// Number of failed tests
    pub failed: usize,
    /// Number of skipped tests
    pub skipped: usize,
    /// Test duration in seconds
    pub duration: f64,
    /// Detailed test output
    pub output: String,
    /// Test failures with details
    pub failures: Vec<TestFailure>,
}

impl TestResults {
    /// Check if all tests passed
    #[must_use]
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }

    /// Get pass rate as percentage
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // Acceptable for percentage calculation
    pub fn pass_rate(&self) -> f64 {
        if self.total == 0 {
            100.0
        } else {
            (self.passed as f64 / self.total as f64) * 100.0
        }
    }
}

/// Details about a test failure
#[derive(Clone, Debug)]
pub struct TestFailure {
    /// Test name
    pub name: String,
    /// Failure message
    pub message: String,
    /// Stack trace or additional details
    pub details: Option<String>,
}

/// Cache configuration for builds
#[derive(Clone, Debug)]
pub struct CacheConfig {
    /// Whether to use ccache/sccache
    pub use_compiler_cache: bool,
    /// Compiler cache type
    pub compiler_cache_type: CompilerCacheType,
    /// Cache directory
    pub cache_dir: PathBuf,
    /// Maximum cache size in bytes
    pub max_size: u64,
    /// Whether to use distributed cache
    pub distributed: bool,
}

/// Type of compiler cache to use
#[derive(Clone, Debug)]
pub enum CompilerCacheType {
    /// Use ccache
    CCache,
    /// Use sccache
    SCCache,
    /// Use distcc
    DistCC,
}
