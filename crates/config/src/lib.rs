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
