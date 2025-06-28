#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Configuration management for sps2
//!
//! This crate handles loading and merging configuration from:
//! - Default values (hard-coded)
//! - Configuration file (~/.config/sps2/config.toml)
//! - Environment variables
//! - CLI flags

use serde::{Deserialize, Serialize};
use sps2_errors::{ConfigError, Error};
use sps2_types::{ColorChoice, OutputFormat};
use std::path::{Path, PathBuf};
use tokio::fs;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,

    #[serde(default)]
    pub build: BuildConfig,

    #[serde(default)]
    pub security: SecurityConfig,

    #[serde(default)]
    pub state: StateConfig,

    #[serde(default)]
    pub paths: PathConfig,

    #[serde(default)]
    pub network: NetworkConfig,

    #[serde(default)]
    pub verification: VerificationConfig,

    #[serde(default)]
    pub guard: Option<GuardConfiguration>,
}

/// General configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_output_format")]
    pub default_output: OutputFormat,
    #[serde(default = "default_color_choice")]
    pub color: ColorChoice,
    #[serde(default = "default_parallel_downloads")]
    pub parallel_downloads: usize,
}

/// Build configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    #[serde(default = "default_build_jobs")]
    pub build_jobs: usize, // 0 = auto-detect
    #[serde(default = "default_network_access")]
    pub network_access: bool,
    #[serde(default = "default_compression_level")]
    pub compression_level: String,
    #[serde(default)]
    pub commands: BuildCommandsConfig,
}

/// Build commands configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildCommandsConfig {
    #[serde(default = "default_allowed_commands")]
    pub allowed: Vec<String>,
    #[serde(default = "default_allowed_shell")]
    pub allowed_shell: Vec<String>,
    #[serde(default)]
    pub additional_allowed: Vec<String>,
    #[serde(default)]
    pub disallowed: Vec<String>,
}

/// Security configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_verify_signatures")]
    pub verify_signatures: bool,
    #[serde(default = "default_allow_unsigned")]
    pub allow_unsigned: bool,
    #[serde(default = "default_index_max_age_days")]
    pub index_max_age_days: u32,
}

/// State management configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateConfig {
    #[serde(default = "default_retention_count")]
    pub retention_count: usize,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

/// Path configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PathConfig {
    pub store_path: Option<PathBuf>,
    pub state_path: Option<PathBuf>,
    pub build_path: Option<PathBuf>,
}

/// Network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64, // seconds
    #[serde(default = "default_retries")]
    pub retries: u32,
    #[serde(default = "default_retry_delay")]
    pub retry_delay: u64, // seconds
}

/// Symlink handling policy for guard operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymlinkPolicyConfig {
    /// Verify symlinks strictly - fail on any symlink issues
    Strict,
    /// Be lenient with bootstrap directories like /opt/pm/live/bin
    LenientBootstrap,
    /// Be lenient with all symlinks - log issues but don't fail
    LenientAll,
    /// Ignore symlinks entirely - skip symlink verification
    Ignore,
}

impl Default for SymlinkPolicyConfig {
    fn default() -> Self {
        Self::LenientBootstrap
    }
}

/// Performance configuration for guard operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct PerformanceConfigToml {
    #[serde(default = "default_use_cache")]
    pub use_cache: bool,
    #[serde(default = "default_parallel_verification")]
    pub parallel_verification: bool,
    #[serde(default = "default_cache_warming")]
    pub cache_warming: bool,
    #[serde(default = "default_progressive_verification")]
    pub progressive_verification: bool,
    #[serde(default = "default_max_concurrent_tasks")]
    pub max_concurrent_tasks: usize,
    #[serde(default = "default_verification_timeout_seconds")]
    pub verification_timeout_seconds: u64,
    #[serde(default = "default_cache_size_mb")]
    pub cache_size_mb: usize,
    #[serde(default = "default_cache_ttl_seconds")]
    pub cache_ttl_seconds: u64,
}

impl Default for PerformanceConfigToml {
    fn default() -> Self {
        Self {
            use_cache: default_use_cache(),
            parallel_verification: default_parallel_verification(),
            cache_warming: default_cache_warming(),
            progressive_verification: default_progressive_verification(),
            max_concurrent_tasks: default_max_concurrent_tasks(),
            verification_timeout_seconds: default_verification_timeout_seconds(),
            cache_size_mb: default_cache_size_mb(),
            cache_ttl_seconds: default_cache_ttl_seconds(),
        }
    }
}

/// Guard-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardConfigToml {
    #[serde(default)]
    pub symlink_policy: SymlinkPolicyConfig,
    #[serde(default = "default_lenient_symlink_directories")]
    pub lenient_symlink_directories: Vec<PathBuf>,
}

impl Default for GuardConfigToml {
    fn default() -> Self {
        Self {
            symlink_policy: SymlinkPolicyConfig::default(),
            lenient_symlink_directories: default_lenient_symlink_directories(),
        }
    }
}

/// Verification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct VerificationConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_verification_level")]
    pub level: String, // "quick", "standard", or "full"
    #[serde(default = "default_fail_on_discrepancy")]
    pub fail_on_discrepancy: bool,
    #[serde(default = "default_auto_heal")]
    pub auto_heal: bool,
    #[serde(default = "default_orphaned_file_action")]
    pub orphaned_file_action: String, // "remove", "preserve", or "backup"
    #[serde(default = "default_orphaned_backup_dir")]
    pub orphaned_backup_dir: PathBuf,
    #[serde(default = "default_preserve_user_files")]
    pub preserve_user_files: bool,

    // Enhanced guard configuration
    #[serde(default)]
    pub guard: GuardConfigToml,
    #[serde(default)]
    pub performance: PerformanceConfigToml,
}

/// Top-level guard configuration (alternative to verification.guard approach)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct GuardConfiguration {
    #[serde(default = "default_guard_enabled")]
    pub enabled: bool,
    #[serde(default = "default_verification_level")]
    pub verification_level: String, // "quick", "standard", or "full"
    #[serde(default = "default_auto_heal")]
    pub auto_heal: bool,
    #[serde(default = "default_fail_on_discrepancy")]
    pub fail_on_discrepancy: bool,
    #[serde(default)]
    pub symlink_policy: GuardSymlinkPolicy,
    #[serde(default = "default_orphaned_file_action")]
    pub orphaned_file_action: String, // "remove", "preserve", or "backup"
    #[serde(default = "default_orphaned_backup_dir")]
    pub orphaned_backup_dir: PathBuf,
    #[serde(default = "default_preserve_user_files")]
    pub preserve_user_files: bool,

    // Nested configuration sections
    #[serde(default)]
    pub performance: GuardPerformanceConfig,
    #[serde(default = "default_guard_lenient_symlink_directories")]
    pub lenient_symlink_directories: Vec<GuardDirectoryConfig>,
}

impl Default for GuardConfiguration {
    fn default() -> Self {
        Self {
            enabled: default_guard_enabled(),
            verification_level: default_verification_level(),
            auto_heal: default_auto_heal(),
            fail_on_discrepancy: default_fail_on_discrepancy(),
            symlink_policy: GuardSymlinkPolicy::default(),
            orphaned_file_action: default_orphaned_file_action(),
            orphaned_backup_dir: default_orphaned_backup_dir(),
            preserve_user_files: default_preserve_user_files(),
            performance: GuardPerformanceConfig::default(),
            lenient_symlink_directories: default_guard_lenient_symlink_directories(),
        }
    }
}

/// Simplified symlink policy for top-level guard configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardSymlinkPolicy {
    /// Verify symlinks strictly - fail on any symlink issues
    Strict,
    /// Be lenient with symlinks - log issues but don't fail
    Lenient,
    /// Ignore symlinks entirely - skip symlink verification
    Ignore,
}

impl Default for GuardSymlinkPolicy {
    fn default() -> Self {
        Self::Lenient
    }
}

/// Performance configuration for top-level guard
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct GuardPerformanceConfig {
    #[serde(default = "default_use_cache")]
    pub use_cache: bool,
    #[serde(default = "default_parallel_verification")]
    pub parallel_verification: bool,
    #[serde(default = "default_cache_warming")]
    pub cache_warming: bool,
    #[serde(default = "default_progressive_verification")]
    pub progressive_verification: bool,
    #[serde(default = "default_max_concurrent_tasks")]
    pub max_concurrent_tasks: usize,
    #[serde(default = "default_verification_timeout_seconds")]
    pub verification_timeout_seconds: u64,
    #[serde(default = "default_cache_size_mb")]
    pub cache_size_mb: usize,
    #[serde(default = "default_cache_ttl_seconds")]
    pub cache_ttl_seconds: u64,
}

impl Default for GuardPerformanceConfig {
    fn default() -> Self {
        Self {
            use_cache: default_use_cache(),
            parallel_verification: default_parallel_verification(),
            cache_warming: default_cache_warming(),
            progressive_verification: default_progressive_verification(),
            max_concurrent_tasks: default_max_concurrent_tasks(),
            verification_timeout_seconds: default_verification_timeout_seconds(),
            cache_size_mb: default_cache_size_mb(),
            cache_ttl_seconds: default_cache_ttl_seconds(),
        }
    }
}

/// Directory configuration for lenient symlink handling (array of tables approach)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardDirectoryConfig {
    pub path: PathBuf,
}

// Default implementations

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_output: OutputFormat::Tty,
            color: ColorChoice::Auto,
            parallel_downloads: 4,
        }
    }
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            build_jobs: 0, // 0 = auto-detect
            network_access: false,
            compression_level: "balanced".to_string(),
            commands: BuildCommandsConfig::default(),
        }
    }
}

impl Default for BuildCommandsConfig {
    fn default() -> Self {
        Self {
            allowed: default_allowed_commands(),
            allowed_shell: default_allowed_shell(),
            additional_allowed: Vec::new(),
            disallowed: Vec::new(),
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            verify_signatures: true,
            allow_unsigned: false,
            index_max_age_days: 7,
        }
    }
}

impl Default for StateConfig {
    fn default() -> Self {
        Self {
            retention_count: 10, // Keep last 10 states
            retention_days: 30,  // Or 30 days, whichever is less
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            timeout: 300, // 5 minutes
            retries: 3,
            retry_delay: 1, // 1 second
        }
    }
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default during development
            level: "standard".to_string(),
            fail_on_discrepancy: true,
            auto_heal: false,
            orphaned_file_action: "preserve".to_string(),
            orphaned_backup_dir: PathBuf::from("/opt/pm/orphaned-backup"),
            preserve_user_files: true,
            guard: GuardConfigToml::default(),
            performance: PerformanceConfigToml::default(),
        }
    }
}

// Default value functions for serde
fn default_output_format() -> OutputFormat {
    OutputFormat::Tty
}

fn default_color_choice() -> ColorChoice {
    ColorChoice::Auto
}

fn default_parallel_downloads() -> usize {
    4
}

fn default_build_jobs() -> usize {
    0 // 0 = auto-detect
}

fn default_network_access() -> bool {
    false
}

fn default_compression_level() -> String {
    "balanced".to_string()
}

fn default_verify_signatures() -> bool {
    true
}

fn default_allow_unsigned() -> bool {
    false
}

fn default_index_max_age_days() -> u32 {
    7
}

fn default_retention_count() -> usize {
    10
}

fn default_retention_days() -> u32 {
    30
}

fn default_timeout() -> u64 {
    300 // 5 minutes
}

fn default_retries() -> u32 {
    3
}

fn default_retry_delay() -> u64 {
    1 // 1 second
}

fn default_enabled() -> bool {
    true // Enable verification by default for state integrity
}

fn default_verification_level() -> String {
    "standard".to_string()
}

fn default_fail_on_discrepancy() -> bool {
    true
}

fn default_auto_heal() -> bool {
    false
}

fn default_orphaned_file_action() -> String {
    "preserve".to_string()
}

fn default_orphaned_backup_dir() -> PathBuf {
    PathBuf::from("/opt/pm/orphaned-backup")
}

fn default_preserve_user_files() -> bool {
    true
}

// Guard configuration defaults

fn default_use_cache() -> bool {
    true
}

fn default_parallel_verification() -> bool {
    true
}

fn default_cache_warming() -> bool {
    true
}

fn default_progressive_verification() -> bool {
    true
}

fn default_max_concurrent_tasks() -> usize {
    8
}

fn default_verification_timeout_seconds() -> u64 {
    300 // 5 minutes
}

fn default_cache_size_mb() -> usize {
    128
}

fn default_cache_ttl_seconds() -> u64 {
    3600 // 1 hour
}

// Top-level guard configuration defaults

fn default_guard_enabled() -> bool {
    true // Enable guard by default for state verification
}

fn default_guard_lenient_symlink_directories() -> Vec<GuardDirectoryConfig> {
    vec![
        GuardDirectoryConfig {
            path: PathBuf::from("/opt/pm/live/bin"),
        },
        GuardDirectoryConfig {
            path: PathBuf::from("/opt/pm/live/sbin"),
        },
    ]
}

fn default_lenient_symlink_directories() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/pm/live/bin"),
        PathBuf::from("/opt/pm/live/sbin"),
    ]
}

fn default_allowed_commands() -> Vec<String> {
    let mut commands = Vec::new();

    // Collect all command categories
    commands.extend(build_tools_commands());
    commands.extend(configure_scripts_commands());
    commands.extend(file_operations_commands());
    commands.extend(text_processing_commands());
    commands.extend(archive_tools_commands());
    commands.extend(shell_builtins_commands());
    commands.extend(development_tools_commands());
    commands.extend(platform_specific_commands());

    commands
}

/// Build tools and compilers
fn build_tools_commands() -> Vec<String> {
    vec![
        "make", "cmake", "meson", "ninja", "cargo", "go", "python", "python3", "pip", "pip3",
        "npm", "yarn", "pnpm", "node", "gcc", "g++", "clang", "clang++", "cc", "c++", "ld", "ar",
        "ranlib", "strip", "objcopy",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Configure scripts and build bootstrappers
fn configure_scripts_commands() -> Vec<String> {
    vec![
        "./configure",
        "configure",
        "./Configure",
        "./config",
        "./bootstrap",
        "./autogen.sh",
        "./buildconf",
        "./waf",
        "./setup.py",
        "./gradlew",
        "./mvnw",
        "./build.sh",
        "./build",
        "./install.sh",
        "./compile",
        "autoreconf",
        "autoconf",
        "automake",
        "libtool",
        "glibtoolize",
        "libtoolize",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// File operations
fn file_operations_commands() -> Vec<String> {
    vec![
        "cp", "mv", "mkdir", "rmdir", "touch", "ln", "install", "chmod", "chown", "rm", "rsync",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Text processing utilities
fn text_processing_commands() -> Vec<String> {
    vec![
        "sed", "awk", "grep", "egrep", "fgrep", "cut", "tr", "sort", "uniq", "head", "tail", "cat",
        "echo", "printf", "test", "[",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Archive and compression tools
fn archive_tools_commands() -> Vec<String> {
    vec![
        "tar", "gzip", "gunzip", "bzip2", "bunzip2", "xz", "unxz", "zip", "unzip",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Shell built-ins and control flow
fn shell_builtins_commands() -> Vec<String> {
    vec![
        "sh",
        "bash",
        "/bin/sh",
        "/bin/bash",
        "cd",
        "pwd",
        "export",
        "source",
        ".",
        "env",
        "set",
        "unset",
        "true",
        "false",
        "if",
        "then",
        "else",
        "elif",
        "fi",
        "for",
        "while",
        "do",
        "done",
        "case",
        "esac",
        "return",
        "exit",
        "shift",
        "break",
        "continue",
        // Version control
        "git",
        "hg",
        "svn",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Development and debugging tools
fn development_tools_commands() -> Vec<String> {
    vec![
        "pkg-config",
        "pkgconf",
        "ldconfig",
        "patch",
        "diff",
        "which",
        "whereis",
        "dirname",
        "basename",
        "readlink",
        "realpath",
        "expr",
        "xargs",
        "tee",
        "time",
        "nproc",
        "getconf",
        "file",
        // Test runners
        "./test.sh",
        "./run-tests.sh",
        "./check.sh",
        "ctest",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Platform-specific tools
fn platform_specific_commands() -> Vec<String> {
    vec![
        // Library inspection
        "ldd",
        "otool",
        "nm",
        "strings",
        "size",
        // macOS specific
        "install_name_tool",
        "codesign",
        "xcrun",
        "lipo",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_allowed_shell() -> Vec<String> {
    vec![
        // Common build patterns
        "mkdir -p",
        "test -f",
        "test -d",
        "test -e",
        "test -x",
        "test -z",
        "test -n",
        "[ -f",
        "[ -d",
        "[ -e",
        "[ -x",
        "[ -z",
        "[ -n",
        "if [",
        "if test",
        "for file in",
        "for dir in",
        "for i in",
        "find . -name",
        "find . -type",
        "echo",
        "printf",
        "cd ${DESTDIR}",
        "cd ${PREFIX}",
        "cd ${BUILD_DIR}",
        "ln -s",
        "ln -sf",
        "cp -r",
        "cp -a",
        "cp -p",
        "install -D",
        "install -m",
        "sed -i",
        "sed -e",
        // Variable assignments
        "export",
        "unset",
        // Conditionals
        "||",
        "&&",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Config {
    /// Get the default config file path
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    pub fn default_path() -> Result<PathBuf, Error> {
        let home_dir = dirs::home_dir().ok_or_else(|| ConfigError::NotFound {
            path: "home directory".to_string(),
        })?;
        Ok(home_dir.join(".config").join("sps2").join("config.toml"))
    }

    /// Load configuration from file
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if the file contents
    /// contain invalid TOML syntax that cannot be parsed.
    pub async fn load_from_file(path: &Path) -> Result<Self, Error> {
        let contents = fs::read_to_string(path)
            .await
            .map_err(|_| ConfigError::NotFound {
                path: path.display().to_string(),
            })?;

        toml::from_str(&contents)
            .map_err(|e| ConfigError::ParseError {
                message: e.to_string(),
            })
            .map_err(Into::into)
    }

    /// Load configuration with fallback to defaults
    ///
    /// If the config file doesn't exist, creates it with defaults.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration file exists but cannot be read
    /// or contains invalid TOML syntax.
    pub async fn load() -> Result<Self, Error> {
        let config_path = Self::default_path()?;

        if config_path.exists() {
            Self::load_from_file(&config_path).await
        } else {
            // Create default config and save it
            let config = Self::default();
            if let Err(e) = config.save().await {
                tracing::warn!("Failed to save default config: {}", e);
            }
            Ok(config)
        }
    }

    /// Load configuration from an optional path or use default
    ///
    /// If path is provided, loads from that file.
    /// If path is None, uses the default loading behavior.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or parsed
    pub async fn load_or_default(path: &Option<std::path::PathBuf>) -> Result<Self, Error> {
        match path {
            Some(config_path) => Self::load_from_file(config_path).await,
            None => Self::load().await,
        }
    }

    /// Merge with environment variables
    ///
    /// # Errors
    ///
    /// Returns an error if environment variables contain invalid values
    /// that cannot be parsed into the expected types.
    pub fn merge_env(&mut self) -> Result<(), Error> {
        // SPS2_OUTPUT
        if let Ok(output) = std::env::var("SPS2_OUTPUT") {
            self.general.default_output = match output.as_str() {
                "plain" => OutputFormat::Plain,
                "tty" => OutputFormat::Tty,
                "json" => OutputFormat::Json,
                _ => {
                    return Err(ConfigError::InvalidValue {
                        field: "SPS2_OUTPUT".to_string(),
                        value: output,
                    }
                    .into())
                }
            };
        }

        // SPS2_COLOR
        if let Ok(color) = std::env::var("SPS2_COLOR") {
            self.general.color = match color.as_str() {
                "always" => ColorChoice::Always,
                "auto" => ColorChoice::Auto,
                "never" => ColorChoice::Never,
                _ => {
                    return Err(ConfigError::InvalidValue {
                        field: "SPS2_COLOR".to_string(),
                        value: color,
                    }
                    .into())
                }
            };
        }

        // SPS2_BUILD_JOBS
        if let Ok(jobs) = std::env::var("SPS2_BUILD_JOBS") {
            self.build.build_jobs = jobs.parse().map_err(|_| ConfigError::InvalidValue {
                field: "SPS2_BUILD_JOBS".to_string(),
                value: jobs,
            })?;
        }

        // SPS2_NETWORK_ACCESS
        if let Ok(network) = std::env::var("SPS2_NETWORK_ACCESS") {
            self.build.network_access = match network.as_str() {
                "true" | "1" | "yes" => true,
                "false" | "0" | "no" => false,
                _ => {
                    return Err(ConfigError::InvalidValue {
                        field: "SPS2_NETWORK_ACCESS".to_string(),
                        value: network,
                    }
                    .into())
                }
            };
        }

        // SPS2_PARALLEL_DOWNLOADS
        if let Ok(downloads) = std::env::var("SPS2_PARALLEL_DOWNLOADS") {
            self.general.parallel_downloads =
                downloads.parse().map_err(|_| ConfigError::InvalidValue {
                    field: "SPS2_PARALLEL_DOWNLOADS".to_string(),
                    value: downloads,
                })?;
        }

        Ok(())
    }

    /// Get the store path (with default)
    #[must_use]
    pub fn store_path(&self) -> PathBuf {
        self.paths
            .store_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("/opt/pm/store"))
    }

    /// Get the state path (with default)
    #[must_use]
    pub fn state_path(&self) -> PathBuf {
        self.paths
            .state_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("/opt/pm/states"))
    }

    /// Get the build path (with default)
    #[must_use]
    pub fn build_path(&self) -> PathBuf {
        self.paths
            .build_path
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// Get the live root path
    #[must_use]
    pub fn live_path(&self) -> PathBuf {
        PathBuf::from("/opt/pm/live")
    }

    /// Get the database path
    #[must_use]
    pub fn db_path(&self) -> PathBuf {
        PathBuf::from("/opt/pm/state.sqlite")
    }

    /// Save configuration to the default location
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be serialized
    /// or if the file cannot be written.
    pub async fn save(&self) -> Result<(), Error> {
        let config_path = Self::default_path()?;
        self.save_to(&config_path).await
    }

    /// Save configuration to a specific path
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be serialized
    /// or if the file cannot be written.
    pub async fn save_to(&self, path: &Path) -> Result<(), Error> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| ConfigError::WriteError {
                    path: parent.display().to_string(),
                    error: e.to_string(),
                })?;
        }

        // Serialize to TOML with pretty formatting
        let toml_string =
            toml::to_string_pretty(self).map_err(|e| ConfigError::SerializeError {
                error: e.to_string(),
            })?;

        // Add header comment
        let content = format!(
            "# sps2 Configuration File\n\
             # This file was automatically generated.\n\
             # You can modify it to customize sps2 behavior.\n\
             #\n\
             # For more information, see: https://github.com/your-org/sps2\n\n\
             {toml_string}"
        );

        // Write to file
        fs::write(path, content)
            .await
            .map_err(|e| ConfigError::WriteError {
                path: path.display().to_string(),
                error: e.to_string(),
            })?;

        Ok(())
    }

    /// Get all allowed commands (combining allowed and `additional_allowed`)
    #[must_use]
    pub fn get_allowed_commands(&self) -> Vec<String> {
        let mut commands = self.build.commands.allowed.clone();
        commands.extend(self.build.commands.additional_allowed.clone());
        commands
    }

    /// Check if a command is allowed
    #[must_use]
    pub fn is_command_allowed(&self, command: &str) -> bool {
        // First check if it's explicitly disallowed
        if self
            .build
            .commands
            .disallowed
            .contains(&command.to_string())
        {
            return false;
        }

        // Then check if it's in the allowed list
        self.get_allowed_commands().contains(&command.to_string())
    }

    /// Check if a shell pattern is allowed
    #[must_use]
    pub fn is_shell_pattern_allowed(&self, pattern: &str) -> bool {
        self.build
            .commands
            .allowed_shell
            .iter()
            .any(|allowed| pattern.starts_with(allowed))
    }

    /// Validate guard configuration settings
    ///
    /// # Errors
    ///
    /// Returns an error if any configuration values are invalid
    pub fn validate_guard_config(&self) -> Result<(), Error> {
        self.validate_verification_config()?;

        if let Some(guard_config) = &self.guard {
            Self::validate_top_level_guard_config(guard_config)?;
        }

        Ok(())
    }

    fn validate_verification_config(&self) -> Result<(), Error> {
        Self::validate_verification_level(&self.verification.level, "verification.level")?;
        Self::validate_orphaned_file_action(
            &self.verification.orphaned_file_action,
            "verification.orphaned_file_action",
        )?;
        Self::validate_toml_performance_config(
            &self.verification.performance,
            "verification.performance",
        )?;
        Self::validate_symlink_directories(
            &self.verification.guard.lenient_symlink_directories,
            "verification.guard.lenient_symlink_directories",
        )?;
        Ok(())
    }

    fn validate_top_level_guard_config(guard_config: &GuardConfiguration) -> Result<(), Error> {
        Self::validate_verification_level(
            &guard_config.verification_level,
            "guard.verification_level",
        )?;
        Self::validate_orphaned_file_action(
            &guard_config.orphaned_file_action,
            "guard.orphaned_file_action",
        )?;
        Self::validate_guard_performance_config(&guard_config.performance, "guard.performance")?;
        Self::validate_guard_symlink_directories(
            &guard_config.lenient_symlink_directories,
            "guard.lenient_symlink_directories",
        )?;
        Ok(())
    }

    fn validate_verification_level(level: &str, field_name: &str) -> Result<(), Error> {
        match level {
            "quick" | "standard" | "full" => Ok(()),
            _ => Err(ConfigError::InvalidValue {
                field: field_name.to_string(),
                value: level.to_string(),
            }
            .into()),
        }
    }

    fn validate_orphaned_file_action(action: &str, field_name: &str) -> Result<(), Error> {
        match action {
            "remove" | "preserve" | "backup" => Ok(()),
            _ => Err(ConfigError::InvalidValue {
                field: field_name.to_string(),
                value: action.to_string(),
            }
            .into()),
        }
    }

    fn validate_toml_performance_config(
        perf: &PerformanceConfigToml,
        field_prefix: &str,
    ) -> Result<(), Error> {
        if perf.max_concurrent_tasks == 0 {
            return Err(ConfigError::InvalidValue {
                field: format!("{field_prefix}.max_concurrent_tasks"),
                value: "0".to_string(),
            }
            .into());
        }

        if perf.verification_timeout_seconds == 0 {
            return Err(ConfigError::InvalidValue {
                field: format!("{field_prefix}.verification_timeout_seconds"),
                value: "0".to_string(),
            }
            .into());
        }

        if perf.cache_size_mb == 0 {
            return Err(ConfigError::InvalidValue {
                field: format!("{field_prefix}.cache_size_mb"),
                value: "0".to_string(),
            }
            .into());
        }

        Ok(())
    }

    fn validate_guard_performance_config(
        perf: &GuardPerformanceConfig,
        field_prefix: &str,
    ) -> Result<(), Error> {
        if perf.max_concurrent_tasks == 0 {
            return Err(ConfigError::InvalidValue {
                field: format!("{field_prefix}.max_concurrent_tasks"),
                value: "0".to_string(),
            }
            .into());
        }

        if perf.verification_timeout_seconds == 0 {
            return Err(ConfigError::InvalidValue {
                field: format!("{field_prefix}.verification_timeout_seconds"),
                value: "0".to_string(),
            }
            .into());
        }

        if perf.cache_size_mb == 0 {
            return Err(ConfigError::InvalidValue {
                field: format!("{field_prefix}.cache_size_mb"),
                value: "0".to_string(),
            }
            .into());
        }

        Ok(())
    }

    fn validate_symlink_directories(dirs: &[PathBuf], field_name: &str) -> Result<(), Error> {
        for dir in dirs {
            if !dir.is_absolute() {
                return Err(ConfigError::InvalidValue {
                    field: field_name.to_string(),
                    value: dir.display().to_string(),
                }
                .into());
            }
        }
        Ok(())
    }

    fn validate_guard_symlink_directories(
        dirs: &[GuardDirectoryConfig],
        field_name: &str,
    ) -> Result<(), Error> {
        for dir_config in dirs {
            if !dir_config.path.is_absolute() {
                return Err(ConfigError::InvalidValue {
                    field: field_name.to_string(),
                    value: dir_config.path.display().to_string(),
                }
                .into());
            }
        }
        Ok(())
    }
}

/// Calculate build jobs based on CPU count
#[must_use]
pub fn calculate_build_jobs(config_value: usize) -> usize {
    if config_value > 0 {
        config_value // User override
    } else {
        // Auto-detect based on CPU count
        let cpus = num_cpus::get();

        // Use 75% of CPUs for builds, minimum 1
        // This leaves headroom for system responsiveness
        (cpus * 3 / 4).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_guard_config_validation() {
        let mut config = Config::default();

        // Test valid configuration
        config.verification.enabled = true;
        config.verification.level = "standard".to_string();
        config.verification.orphaned_file_action = "preserve".to_string();
        config.verification.performance.max_concurrent_tasks = 8;
        config.verification.performance.verification_timeout_seconds = 300;
        config.verification.performance.cache_size_mb = 128;
        config.verification.guard.lenient_symlink_directories = vec![
            PathBuf::from("/opt/pm/live/bin"),
            PathBuf::from("/opt/pm/live/sbin"),
        ];

        assert!(config.validate_guard_config().is_ok());

        // Test invalid verification level
        config.verification.level = "invalid".to_string();
        assert!(config.validate_guard_config().is_err());

        // Reset and test invalid orphaned file action
        config.verification.level = "standard".to_string();
        config.verification.orphaned_file_action = "invalid".to_string();
        assert!(config.validate_guard_config().is_err());

        // Reset and test invalid max concurrent tasks
        config.verification.orphaned_file_action = "preserve".to_string();
        config.verification.performance.max_concurrent_tasks = 0;
        assert!(config.validate_guard_config().is_err());

        // Reset and test invalid relative path
        config.verification.performance.max_concurrent_tasks = 8;
        config.verification.guard.lenient_symlink_directories =
            vec![PathBuf::from("relative/path")];
        assert!(config.validate_guard_config().is_err());
    }

    #[test]
    fn test_symlink_policy_serialization() {
        // Test that symlink policies serialize/deserialize correctly in guard config
        #[derive(serde::Serialize, serde::Deserialize)]
        struct TestConfig {
            symlink_policy: SymlinkPolicyConfig,
        }

        let test_config = TestConfig {
            symlink_policy: SymlinkPolicyConfig::LenientBootstrap,
        };
        let serialized = toml::to_string(&test_config).unwrap();
        let deserialized: TestConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(test_config.symlink_policy, deserialized.symlink_policy);
        assert!(serialized.contains("symlink_policy = \"lenient_bootstrap\""));
    }

    #[tokio::test]
    async fn test_config_toml_generation() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_config.toml");

        let mut config = Config::default();
        config.verification.enabled = true;
        config.verification.guard.symlink_policy = SymlinkPolicyConfig::LenientBootstrap;
        config.verification.performance.parallel_verification = true;
        config.verification.performance.cache_warming = true;
        config.verification.performance.max_concurrent_tasks = 6;

        // Save config
        config.save_to(&config_path).await.unwrap();

        // Verify file was created and contains expected content
        let contents = tokio::fs::read_to_string(&config_path).await.unwrap();
        assert!(contents.contains("[verification.guard]"));
        assert!(contents.contains("[verification.performance]"));
        assert!(contents.contains("symlink_policy = \"lenient_bootstrap\""));
        assert!(contents.contains("parallel_verification = true"));
        assert!(contents.contains("max_concurrent_tasks = 6"));

        // Load config back and verify it matches
        let loaded_config = Config::load_from_file(&config_path).await.unwrap();
        assert!(loaded_config.verification.enabled);
        assert_eq!(
            loaded_config.verification.guard.symlink_policy,
            SymlinkPolicyConfig::LenientBootstrap
        );
        assert!(loaded_config.verification.performance.parallel_verification);
        assert_eq!(
            loaded_config.verification.performance.max_concurrent_tasks,
            6
        );
    }

    #[test]
    fn test_config_defaults() {
        let config = Config::default();

        // Test default verification config
        assert!(!config.verification.enabled);
        assert_eq!(config.verification.level, "standard");
        assert!(!config.verification.auto_heal);
        assert!(config.verification.fail_on_discrepancy);

        // Test default guard config
        assert_eq!(
            config.verification.guard.symlink_policy,
            SymlinkPolicyConfig::LenientBootstrap
        );
        assert_eq!(
            config.verification.guard.lenient_symlink_directories,
            vec![
                PathBuf::from("/opt/pm/live/bin"),
                PathBuf::from("/opt/pm/live/sbin"),
            ]
        );

        // Test default performance config
        assert!(config.verification.performance.use_cache);
        assert!(config.verification.performance.parallel_verification);
        assert!(config.verification.performance.cache_warming);
        assert!(config.verification.performance.progressive_verification);
        assert_eq!(config.verification.performance.max_concurrent_tasks, 8);
        assert_eq!(
            config.verification.performance.verification_timeout_seconds,
            300
        );
        assert_eq!(config.verification.performance.cache_size_mb, 128);
        assert_eq!(config.verification.performance.cache_ttl_seconds, 3600);
    }

    #[tokio::test]
    async fn test_top_level_guard_configuration() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_guard_config.toml");

        // Create a config with top-level [guard] section
        let config = Config {
            guard: Some(GuardConfiguration {
                enabled: true,
                verification_level: "full".to_string(),
                auto_heal: true,
                fail_on_discrepancy: false,
                symlink_policy: GuardSymlinkPolicy::Strict,
                orphaned_file_action: "backup".to_string(),
                orphaned_backup_dir: PathBuf::from("/tmp/backup"),
                preserve_user_files: false,
                performance: GuardPerformanceConfig {
                    use_cache: false,
                    parallel_verification: false,
                    cache_warming: false,
                    progressive_verification: false,
                    max_concurrent_tasks: 4,
                    verification_timeout_seconds: 600,
                    cache_size_mb: 64,
                    cache_ttl_seconds: 1800,
                },
                lenient_symlink_directories: vec![GuardDirectoryConfig {
                    path: PathBuf::from("/custom/path"),
                }],
            }),
            ..Default::default()
        };

        // Save and reload config
        config.save_to(&config_path).await.unwrap();
        let loaded_config = Config::load_from_file(&config_path).await.unwrap();

        // Verify top-level guard configuration
        let guard_config = loaded_config.guard.as_ref().unwrap();
        assert!(guard_config.enabled);
        assert_eq!(guard_config.verification_level, "full");
        assert!(guard_config.auto_heal);
        assert!(!guard_config.fail_on_discrepancy);
        assert_eq!(guard_config.symlink_policy, GuardSymlinkPolicy::Strict);
        assert_eq!(guard_config.orphaned_file_action, "backup");
        assert_eq!(
            guard_config.orphaned_backup_dir,
            PathBuf::from("/tmp/backup")
        );
        assert!(!guard_config.preserve_user_files);

        // Verify performance config
        assert!(!guard_config.performance.use_cache);
        assert!(!guard_config.performance.parallel_verification);
        assert_eq!(guard_config.performance.max_concurrent_tasks, 4);
        assert_eq!(guard_config.performance.verification_timeout_seconds, 600);

        // Verify directory config
        assert_eq!(guard_config.lenient_symlink_directories.len(), 1);
        assert_eq!(
            guard_config.lenient_symlink_directories[0].path,
            PathBuf::from("/custom/path")
        );
    }

    #[tokio::test]
    async fn test_top_level_guard_toml_generation() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test_guard_toml.toml");

        let config = Config {
            guard: Some(GuardConfiguration {
                enabled: true,
                verification_level: "standard".to_string(),
                auto_heal: false,
                fail_on_discrepancy: true,
                symlink_policy: GuardSymlinkPolicy::Lenient,
                orphaned_file_action: "preserve".to_string(),
                orphaned_backup_dir: PathBuf::from("/opt/pm/orphaned-backup"),
                preserve_user_files: true,
                performance: GuardPerformanceConfig::default(),
                lenient_symlink_directories: vec![
                    GuardDirectoryConfig {
                        path: PathBuf::from("/opt/pm/live/bin"),
                    },
                    GuardDirectoryConfig {
                        path: PathBuf::from("/opt/pm/live/sbin"),
                    },
                ],
            }),
            ..Default::default()
        };

        // Save config
        config.save_to(&config_path).await.unwrap();

        // Verify file contains expected TOML structure
        let contents = tokio::fs::read_to_string(&config_path).await.unwrap();
        assert!(contents.contains("[guard]"));
        assert!(contents.contains("enabled = true"));
        assert!(contents.contains("verification_level = \"standard\""));
        assert!(contents.contains("symlink_policy = \"lenient\""));
        assert!(contents.contains("[guard.performance]"));
        assert!(contents.contains("use_cache = true"));
        assert!(contents.contains("[[guard.lenient_symlink_directories]]"));
        assert!(contents.contains("path = \"/opt/pm/live/bin\""));
        assert!(contents.contains("path = \"/opt/pm/live/sbin\""));
    }

    #[test]
    fn test_top_level_guard_validation() {
        let mut config = Config {
            guard: Some(GuardConfiguration::default()),
            ..Default::default()
        };

        // Test valid configuration
        config.guard.as_mut().unwrap().enabled = true;
        config.guard.as_mut().unwrap().verification_level = "standard".to_string();
        config.guard.as_mut().unwrap().orphaned_file_action = "preserve".to_string();
        config
            .guard
            .as_mut()
            .unwrap()
            .performance
            .max_concurrent_tasks = 8;
        config.guard.as_mut().unwrap().lenient_symlink_directories = vec![GuardDirectoryConfig {
            path: PathBuf::from("/opt/pm/live/bin"),
        }];

        assert!(config.validate_guard_config().is_ok());

        // Test invalid verification level
        config.guard.as_mut().unwrap().verification_level = "invalid".to_string();
        assert!(config.validate_guard_config().is_err());

        // Reset and test invalid orphaned file action
        config.guard.as_mut().unwrap().verification_level = "standard".to_string();
        config.guard.as_mut().unwrap().orphaned_file_action = "invalid".to_string();
        assert!(config.validate_guard_config().is_err());

        // Reset and test invalid max concurrent tasks
        config.guard.as_mut().unwrap().orphaned_file_action = "preserve".to_string();
        config
            .guard
            .as_mut()
            .unwrap()
            .performance
            .max_concurrent_tasks = 0;
        assert!(config.validate_guard_config().is_err());

        // Reset and test invalid relative path
        config
            .guard
            .as_mut()
            .unwrap()
            .performance
            .max_concurrent_tasks = 8;
        config.guard.as_mut().unwrap().lenient_symlink_directories = vec![GuardDirectoryConfig {
            path: PathBuf::from("relative/path"),
        }];
        assert!(config.validate_guard_config().is_err());
    }

    #[test]
    fn test_guard_symlink_policy_serialization() {
        // Test that GuardSymlinkPolicy serializes/deserializes correctly
        #[derive(serde::Serialize, serde::Deserialize)]
        struct TestConfig {
            symlink_policy: GuardSymlinkPolicy,
        }

        let test_config = TestConfig {
            symlink_policy: GuardSymlinkPolicy::Lenient,
        };
        let serialized = toml::to_string(&test_config).unwrap();
        let deserialized: TestConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(test_config.symlink_policy, deserialized.symlink_policy);
        assert!(serialized.contains("symlink_policy = \"lenient\""));
    }

    #[test]
    fn test_guard_defaults() {
        let guard_config = GuardConfiguration::default();

        // Test default values
        assert!(!guard_config.enabled);
        assert_eq!(guard_config.verification_level, "standard");
        assert!(!guard_config.auto_heal);
        assert!(guard_config.fail_on_discrepancy);
        assert_eq!(guard_config.symlink_policy, GuardSymlinkPolicy::Lenient);
        assert_eq!(guard_config.orphaned_file_action, "preserve");
        assert!(guard_config.preserve_user_files);

        // Test default performance config
        assert!(guard_config.performance.use_cache);
        assert!(guard_config.performance.parallel_verification);
        assert!(guard_config.performance.cache_warming);
        assert!(guard_config.performance.progressive_verification);
        assert_eq!(guard_config.performance.max_concurrent_tasks, 8);
        assert_eq!(guard_config.performance.verification_timeout_seconds, 300);
        assert_eq!(guard_config.performance.cache_size_mb, 128);
        assert_eq!(guard_config.performance.cache_ttl_seconds, 3600);

        // Test default directories
        assert_eq!(guard_config.lenient_symlink_directories.len(), 2);
        assert_eq!(
            guard_config.lenient_symlink_directories[0].path,
            PathBuf::from("/opt/pm/live/bin")
        );
        assert_eq!(
            guard_config.lenient_symlink_directories[1].path,
            PathBuf::from("/opt/pm/live/sbin")
        );
    }
}
