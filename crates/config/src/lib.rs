#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Configuration management for sps2
//!
//! This crate handles loading and merging configuration from:
//! - Default values (hard-coded)
//! - Configuration file (~/.config/sps2/config.toml)
//! - Builder configuration file (~/.config/sps2/builder.config.toml)
//! - Environment variables
//! - CLI flags

pub mod builder;
pub mod constants;
pub mod core;
pub mod guard;
pub mod repository;

// Re-export main types for convenience
pub use builder::BuilderConfig;
pub use constants as fixed_paths;
pub use core::{GeneralConfig, NetworkConfig, PathConfig, SecurityConfig, StateConfig};
pub use guard::{
    DiscrepancyHandling, GuardConfiguration, GuardDirectoryConfig, GuardPerformanceConfig,
    GuardSymlinkPolicy, PerformanceConfigToml, SymlinkPolicyConfig, UserFilePolicy,
    VerificationConfig,
};
pub use repository::{Repositories, RepositoryConfig};

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

    /// Builder configuration (loaded from separate file)
    #[serde(skip)]
    pub builder: BuilderConfig,

    /// Repository definitions (fast/slow/stable/extras)
    #[serde(default)]
    pub repos: repository::Repositories,

    /// Content-addressable store cleanup policy
    #[serde(default)]
    pub cas: core::CasConfig,
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

        let mut config: Self = toml::from_str(&contents).map_err(|e| ConfigError::ParseError {
            message: e.to_string(),
        })?;

        // Apply legacy field conversions
        config.verification.apply_legacy_fields();
        if let Some(ref mut guard) = config.guard {
            guard.apply_legacy_fields();
        }

        // Load builder config
        config.builder = BuilderConfig::load().await?;

        Ok(config)
    }

    /// Load configuration from file with custom builder config path
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if the file contents
    /// contain invalid TOML syntax that cannot be parsed.
    pub async fn load_from_file_with_builder(
        path: &Path,
        builder_path: &Option<PathBuf>,
    ) -> Result<Self, Error> {
        let contents = fs::read_to_string(path)
            .await
            .map_err(|_| ConfigError::NotFound {
                path: path.display().to_string(),
            })?;

        let mut config: Self = toml::from_str(&contents).map_err(|e| ConfigError::ParseError {
            message: e.to_string(),
        })?;

        // Apply legacy field conversions
        config.verification.apply_legacy_fields();
        if let Some(ref mut guard) = config.guard {
            guard.apply_legacy_fields();
        }

        // Load builder config
        config.builder = BuilderConfig::load_or_default(builder_path).await?;

        Ok(config)
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
            let builder = BuilderConfig::load().await?;
            let config = Self {
                builder,
                ..Self::default()
            };
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

    /// Load configuration from optional paths or use defaults
    ///
    /// If `config_path` is provided, loads from that file.
    /// If `builder_path` is provided, loads builder config from that file.
    /// If paths are None, uses the default loading behavior.
    ///
    /// # Errors
    ///
    /// Returns an error if the config files cannot be read or parsed
    pub async fn load_or_default_with_builder(
        config_path: &Option<std::path::PathBuf>,
        builder_path: &Option<std::path::PathBuf>,
    ) -> Result<Self, Error> {
        if let Some(path) = config_path {
            Self::load_from_file_with_builder(path, builder_path).await
        } else {
            let mut config = Self::load().await?;
            // Override builder config if custom path provided
            if builder_path.is_some() {
                config.builder = BuilderConfig::load_or_default(builder_path).await?;
            }
            Ok(config)
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
            self.builder.build.build_jobs =
                jobs.parse().map_err(|_| ConfigError::InvalidValue {
                    field: "SPS2_BUILD_JOBS".to_string(),
                    value: jobs,
                })?;
        }

        // SPS2_NETWORK_ACCESS removed - network access comes from recipe, not config

        // SPS2_PARALLEL_DOWNLOADS
        if let Ok(downloads) = std::env::var("SPS2_PARALLEL_DOWNLOADS") {
            self.general.parallel_downloads =
                downloads.parse().map_err(|_| ConfigError::InvalidValue {
                    field: "SPS2_PARALLEL_DOWNLOADS".to_string(),
                    value: downloads,
                })?;
        }

        // Optional CAS env overrides (best-effort; ignore if invalid)
        if let Ok(v) = std::env::var("SPS2_CAS_KEEP_STATES") {
            if let Ok(n) = v.parse() {
                self.cas.keep_states_count = n;
            }
        }
        if let Ok(v) = std::env::var("SPS2_CAS_KEEP_DAYS") {
            if let Ok(n) = v.parse() {
                self.cas.keep_days = n;
            }
        }
        if let Ok(v) = std::env::var("SPS2_CAS_PKG_GRACE_DAYS") {
            if let Ok(n) = v.parse() {
                self.cas.package_grace_days = n;
            }
        }
        if let Ok(v) = std::env::var("SPS2_CAS_OBJ_GRACE_DAYS") {
            if let Ok(n) = v.parse() {
                self.cas.object_grace_days = n;
            }
        }
        if let Ok(v) = std::env::var("SPS2_CAS_DRY_RUN") {
            self.cas.dry_run = matches!(v.as_str(), "1" | "true" | "yes");
        }

        Ok(())
    }

    /// Get the store path (with default)
    #[must_use]
    pub fn store_path(&self) -> PathBuf {
        self.paths
            .store_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(crate::constants::STORE_DIR))
    }

    /// Get the state path (with default)
    #[must_use]
    pub fn state_path(&self) -> PathBuf {
        self.paths
            .state_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(crate::constants::STATES_DIR))
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
        PathBuf::from(crate::constants::LIVE_DIR)
    }

    /// Get the database path
    #[must_use]
    pub fn db_path(&self) -> PathBuf {
        PathBuf::from(crate::constants::DB_PATH)
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

    /// Get all allowed commands (delegated to builder config)
    #[must_use]
    pub fn get_allowed_commands(&self) -> Vec<String> {
        self.builder.get_allowed_commands()
    }

    /// Check if a command is allowed (delegated to builder config)
    #[must_use]
    pub fn is_command_allowed(&self, command: &str) -> bool {
        self.builder.is_command_allowed(command)
    }

    /// Check if a shell pattern is allowed (delegated to builder config)
    #[must_use]
    pub fn is_shell_pattern_allowed(&self, pattern: &str) -> bool {
        self.builder.is_shell_pattern_allowed(pattern)
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
        perf: &guard::PerformanceConfigToml,
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

        Ok(())
    }

    fn validate_guard_performance_config(
        perf: &guard::GuardPerformanceConfig,
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
        dirs: &[guard::GuardDirectoryConfig],
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
