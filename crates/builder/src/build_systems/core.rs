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
    /// Cross-compilation context (if any)
    pub cross_compilation: Option<CrossCompilationContext>,
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
            cross_compilation: None,
            extra_env: Arc::new(RwLock::new(HashMap::new())),
            network_allowed: false,
            cache_config: None,
        }
    }

    /// Set build directory for out-of-source builds
    pub fn with_build_dir(mut self, build_dir: PathBuf) -> Self {
        self.build_dir = build_dir;
        self
    }

    /// Set cross-compilation context
    pub fn with_cross_compilation(mut self, cross: CrossCompilationContext) -> Self {
        self.cross_compilation = Some(cross);
        self
    }

    /// Set up enhanced cross-compilation context
    ///
    /// # Errors
    ///
    /// Returns an error if cross-compilation setup fails
    pub async fn setup_cross_compilation(
        mut self,
        build_triple: &str,
        host_triple: &str,
        target_triple: Option<&str>,
        sysroot: PathBuf,
        event_sender: Option<crate::EventSender>,
    ) -> Result<Self, Error> {
        // Create enhanced context
        let enhanced = crate::cross::EnhancedCrossContext::new(
            build_triple,
            host_triple,
            target_triple,
            sysroot,
            event_sender,
        )
        .await?;

        // Add enhanced environment variables first
        let cross_vars = enhanced.get_cross_env_vars();
        if let Ok(mut extra) = self.extra_env.write() {
            extra.extend(cross_vars);
        }

        // Then extract base context
        self.cross_compilation = Some(enhanced.base);

        Ok(self)
    }

    /// Add extra environment variables
    pub fn with_extra_env(mut self, env: HashMap<String, String>) -> Self {
        self.extra_env = Arc::new(RwLock::new(env));
        self
    }

    /// Set network access permission
    pub fn with_network_allowed(mut self, allowed: bool) -> Self {
        self.network_allowed = allowed;
        self
    }

    /// Set cache configuration
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

        // Add cross-compilation variables if applicable
        if let Some(cross) = &self.cross_compilation {
            vars.extend(cross.get_cross_env_vars());
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
            cross_compilation: self.cross_compilation.clone(),
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
            .field("cross_compilation", &self.cross_compilation)
            .field("extra_env", &self.extra_env)
            .field("network_allowed", &self.network_allowed)
            .field("cache_config", &self.cache_config)
            .finish()
    }
}

/// Cross-compilation context
#[derive(Clone, Debug)]
pub struct CrossCompilationContext {
    /// Build platform (where compilation happens)
    pub build_platform: Platform,
    /// Host platform (where the package will run)
    pub host_platform: Platform,
    /// Target platform (what the package targets, for compilers)
    pub target_platform: Option<Platform>,
    /// Sysroot for target platform
    pub sysroot: PathBuf,
    /// Cross-compilation toolchain
    pub toolchain: Toolchain,
}

impl CrossCompilationContext {
    /// Get environment variables for cross-compilation
    pub fn get_cross_env_vars(&self) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        // Toolchain variables
        vars.insert("CC".to_string(), self.toolchain.cc.clone());
        vars.insert("CXX".to_string(), self.toolchain.cxx.clone());
        vars.insert("AR".to_string(), self.toolchain.ar.clone());
        vars.insert("STRIP".to_string(), self.toolchain.strip.clone());
        vars.insert("RANLIB".to_string(), self.toolchain.ranlib.clone());

        // Sysroot and paths
        vars.insert("SYSROOT".to_string(), self.sysroot.display().to_string());
        vars.insert(
            "PKG_CONFIG_SYSROOT_DIR".to_string(),
            self.sysroot.display().to_string(),
        );
        vars.insert(
            "PKG_CONFIG_LIBDIR".to_string(),
            format!("{}/usr/lib/pkgconfig", self.sysroot.display()),
        );

        // Platform-specific variables
        vars.insert(
            "CMAKE_TOOLCHAIN_FILE".to_string(),
            self.toolchain.cmake_toolchain_file.display().to_string(),
        );
        vars.insert(
            "MESON_CROSS_FILE".to_string(),
            self.toolchain.meson_cross_file.display().to_string(),
        );

        vars
    }
}

/// Platform information
#[derive(Clone, Debug)]
pub struct Platform {
    /// Architecture (e.g., "aarch64", "x86_64")
    pub arch: String,
    /// Operating system (e.g., "darwin", "linux")
    pub os: String,
    /// ABI (e.g., "gnu", "musl")
    pub abi: Option<String>,
    /// Vendor (e.g., "apple", "unknown")
    pub vendor: Option<String>,
}

impl Platform {
    /// Get target triple
    pub fn triple(&self) -> String {
        let mut parts = vec![self.arch.clone()];

        if let Some(vendor) = &self.vendor {
            parts.push(vendor.clone());
        } else {
            parts.push("unknown".to_string());
        }

        parts.push(self.os.clone());

        if let Some(abi) = &self.abi {
            parts.push(abi.clone());
        }

        parts.join("-")
    }
}

/// Cross-compilation toolchain
#[derive(Clone, Debug)]
pub struct Toolchain {
    /// C compiler
    pub cc: String,
    /// C++ compiler
    pub cxx: String,
    /// Archiver
    pub ar: String,
    /// Strip utility
    pub strip: String,
    /// Ranlib utility
    pub ranlib: String,
    /// CMake toolchain file
    pub cmake_toolchain_file: PathBuf,
    /// Meson cross file
    pub meson_cross_file: PathBuf,
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
