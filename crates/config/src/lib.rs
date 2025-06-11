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

impl Config {
    /// Get the default config file path
    ///
    /// # Errors
    ///
    /// Returns an error if the system config directory cannot be determined.
    pub fn default_path() -> Result<PathBuf, Error> {
        let config_dir = dirs::config_dir().ok_or_else(|| ConfigError::NotFound {
            path: "config directory".to_string(),
        })?;
        Ok(config_dir.join("sps2").join("config.toml"))
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
    /// # Errors
    ///
    /// Returns an error if the configuration file exists but cannot be read
    /// or contains invalid TOML syntax.
    pub async fn load() -> Result<Self, Error> {
        let config_path = Self::default_path()?;

        if config_path.exists() {
            Self::load_from_file(&config_path).await
        } else {
            Ok(Self::default())
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
