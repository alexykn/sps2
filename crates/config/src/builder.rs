//! Builder configuration for package building and compilation

use serde::{de::IgnoredAny, Deserialize, Serialize};
use sps2_errors::{ConfigError, Error};
use std::path::{Path, PathBuf};
use tokio::fs;

/// Complete builder configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuilderConfig {
    #[serde(default)]
    pub build: BuildSettings,
    #[serde(default)]
    pub packaging: PackagingSettings,
    #[serde(default)]
    pub environment: EnvironmentSettings,
    #[serde(default)]
    pub performance: PerformanceSettings,
    #[serde(default)]
    pub security: SecuritySettings,
}

/// Core build execution settings (global defaults and policies)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSettings {
    #[serde(default = "default_build_jobs")]
    pub build_jobs: usize, // 0 = auto-detect, can be overridden per recipe
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64, // Default timeout, can be overridden per recipe
    #[serde(default = "default_build_root")]
    pub build_root: PathBuf, // Global build directory
    #[serde(default = "default_cleanup_on_success")]
    pub cleanup_on_success: bool,
    #[serde(default = "default_strict_mode")]
    pub strict_mode: bool,
    // Recipe environment defaults (can be overridden per recipe)
    #[serde(default = "default_isolation_level")]
    pub default_isolation_level: String, // "none", "default", "enhanced", "hermetic"
    #[serde(default = "default_allow_network")]
    pub default_allow_network: bool, // Default network access policy
}

impl Default for BuildSettings {
    fn default() -> Self {
        Self {
            build_jobs: 0,         // 0 = auto-detect
            timeout_seconds: 3600, // 1 hour
            build_root: PathBuf::from("/opt/pm/build"),
            cleanup_on_success: true,
            strict_mode: true,
            default_isolation_level: "default".to_string(),
            default_allow_network: false,
        }
    }
}

/// Package generation settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackagingSettings {
    #[serde(default)]
    pub sbom: SbomSettings,
    #[serde(default)]
    pub signing: SigningSettings,
    /// Legacy compression configuration retained for backward compatibility
    #[serde(default, alias = "compression", skip_serializing)]
    pub legacy_compression: Option<IgnoredAny>,
}

/// SBOM (Software Bill of Materials) generation settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbomSettings {
    #[serde(default = "default_sbom_enabled")]
    pub enabled: bool,
    #[serde(default = "default_sbom_format")]
    pub format: String, // "spdx-json", "cyclone-dx", etc.
    #[serde(default = "default_include_build_info")]
    pub include_build_info: bool,
    #[serde(default = "default_include_dependencies")]
    pub include_dependencies: bool,
    #[serde(default)]
    pub exclusions: Vec<String>,
}

impl Default for SbomSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            format: "spdx-json".to_string(),
            include_build_info: true,
            include_dependencies: true,
            exclusions: Vec::new(),
        }
    }
}

/// Code signing settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningSettings {
    #[serde(default = "default_signing_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub identity: Option<String>,
    #[serde(default)]
    pub keychain_path: Option<PathBuf>,
    #[serde(default = "default_enable_hardened_runtime")]
    pub enable_hardened_runtime: bool,
    #[serde(default)]
    pub entitlements_file: Option<PathBuf>,
}

impl Default for SigningSettings {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default
            identity: None,
            keychain_path: None,
            enable_hardened_runtime: true,
            entitlements_file: None,
        }
    }
}

/// Environment constraints and policies for hermetic builds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentSettings {
    #[serde(default)]
    pub allowed_env_vars: Vec<String>,
    #[serde(default)]
    pub allowed_read_paths: Vec<PathBuf>,
    #[serde(default)]
    pub allowed_write_paths: Vec<PathBuf>,
    #[serde(default)]
    pub custom_env_vars: std::collections::HashMap<String, String>,
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self {
            allowed_env_vars: default_allowed_env_vars(),
            allowed_read_paths: default_allowed_read_paths(),
            allowed_write_paths: default_allowed_write_paths(),
            custom_env_vars: std::collections::HashMap::new(),
        }
    }
}

/// Performance and caching settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PerformanceSettings {
    #[serde(default)]
    pub cache: CacheSettings,
    #[serde(default)]
    pub build_system: BuildSystemSettings,
}

/// Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheSettings {
    #[serde(default = "default_cache_enabled")]
    pub enabled: bool,
    #[serde(default = "default_cache_type")]
    pub cache_type: String, // "ccache", "sccache", "none"
    #[serde(default = "default_cache_dir")]
    pub cache_dir: Option<PathBuf>,
    #[serde(default = "default_cache_size_mb")]
    pub max_size_mb: u64,
    #[serde(default = "default_distributed_cache")]
    pub distributed: bool,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_type: "ccache".to_string(),
            cache_dir: default_cache_dir(), // Auto-detect
            max_size_mb: 5000,              // 5GB
            distributed: false,
        }
    }
}

/// Build system specific settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSystemSettings {
    #[serde(default = "default_parallel_builds")]
    pub parallel_builds: bool,
    #[serde(default = "default_out_of_source")]
    pub prefer_out_of_source: bool,
    #[serde(default)]
    pub cmake_args: Vec<String>,
    #[serde(default)]
    pub configure_args: Vec<String>,
    #[serde(default)]
    pub make_args: Vec<String>,
}

impl Default for BuildSystemSettings {
    fn default() -> Self {
        Self {
            parallel_builds: true,
            prefer_out_of_source: true,
            cmake_args: Vec::new(),
            configure_args: Vec::new(),
            make_args: Vec::new(),
        }
    }
}

/// Security and validation settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecuritySettings {
    #[serde(default)]
    pub commands: CommandsConfig,
    #[serde(default)]
    pub validation: ValidationConfig,
}

/// Build commands configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsConfig {
    #[serde(default = "default_allowed_commands")]
    pub allowed: Vec<String>,
    #[serde(default = "default_allowed_shell")]
    pub allowed_shell: Vec<String>,
    #[serde(default)]
    pub additional_allowed: Vec<String>,
    #[serde(default)]
    pub disallowed: Vec<String>,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            allowed: default_allowed_commands(),
            allowed_shell: default_allowed_shell(),
            additional_allowed: Vec::new(),
            disallowed: Vec::new(),
        }
    }
}

/// Validation configuration for build commands
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    #[serde(default)]
    pub mode: ValidationMode,
    #[serde(default)]
    pub shell_expansion: ShellExpansionPolicy,
    #[serde(default)]
    pub path_validation: ValidationMode,
    #[serde(default)]
    pub signature_validation: ValidationMode,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            mode: ValidationMode::Strict,
            shell_expansion: ShellExpansionPolicy::Disabled,
            path_validation: ValidationMode::Strict,
            signature_validation: ValidationMode::Strict,
        }
    }
}

/// Validation mode for build operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationMode {
    /// Strict validation - fail on any validation errors
    Strict,
    /// Lenient validation - log warnings but continue
    Lenient,
    /// Disabled validation - skip all validation checks
    Disabled,
}

impl Default for ValidationMode {
    fn default() -> Self {
        Self::Strict
    }
}

/// Shell expansion policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellExpansionPolicy {
    /// Allow shell expansion
    Enabled,
    /// Disable shell expansion
    Disabled,
}

impl Default for ShellExpansionPolicy {
    fn default() -> Self {
        Self::Disabled
    }
}

impl BuilderConfig {
    /// Get the default builder config file path
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    pub fn default_path() -> Result<PathBuf, Error> {
        let home_dir = dirs::home_dir().ok_or_else(|| ConfigError::NotFound {
            path: "home directory".to_string(),
        })?;
        Ok(home_dir
            .join(".config")
            .join("sps2")
            .join("builder.config.toml"))
    }

    /// Load builder configuration from file
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

        toml::from_str(&contents).map_err(|e| {
            ConfigError::ParseError {
                message: e.to_string(),
            }
            .into()
        })
    }

    /// Load builder configuration with fallback to defaults
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
                tracing::warn!("Failed to save default builder config: {}", e);
            }
            Ok(config)
        }
    }

    /// Load builder configuration from an optional path or use default
    ///
    /// If path is provided, loads from that file.
    /// If path is None, uses the default loading behavior.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or parsed
    pub async fn load_or_default(path: &Option<PathBuf>) -> Result<Self, Error> {
        match path {
            Some(config_path) => Self::load_from_file(config_path).await,
            None => Self::load().await,
        }
    }

    /// Save builder configuration to the default location
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be serialized
    /// or if the file cannot be written.
    pub async fn save(&self) -> Result<(), Error> {
        let config_path = Self::default_path()?;
        self.save_to(&config_path).await
    }

    /// Save builder configuration to a specific path
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
            "# sps2 Builder Configuration File\n\
             # This file was automatically generated.\n\
             # You can modify it to customize build behavior and security settings.\n\
             # This file contains build commands, validation settings, and security policies.\n\n\
             {toml_string}"
        );

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
        let mut commands = self.security.commands.allowed.clone();
        commands.extend(self.security.commands.additional_allowed.clone());
        commands
    }

    /// Check if a command is allowed
    #[must_use]
    pub fn is_command_allowed(&self, command: &str) -> bool {
        // First check if it's explicitly disallowed
        if self
            .security
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
        self.security
            .commands
            .allowed_shell
            .iter()
            .any(|allowed| pattern.starts_with(allowed))
    }

    /// Validate all configuration settings
    ///
    /// # Errors
    ///
    /// Returns an error if any configuration values are invalid
    pub fn validate(&self) -> Result<(), Error> {
        // Validate build settings
        if self.build.timeout_seconds == 0 {
            return Err(ConfigError::InvalidValue {
                field: "build.timeout_seconds".to_string(),
                value: "0".to_string(),
            }
            .into());
        }

        for path in &self.environment.allowed_read_paths {
            if !path.is_absolute() {
                return Err(ConfigError::InvalidValue {
                    field: "environment.allowed_read_paths".to_string(),
                    value: path.display().to_string(),
                }
                .into());
            }
        }

        for path in &self.environment.allowed_write_paths {
            if !path.is_absolute() {
                return Err(ConfigError::InvalidValue {
                    field: "environment.allowed_write_paths".to_string(),
                    value: path.display().to_string(),
                }
                .into());
            }
        }

        Ok(())
    }
}

// Default value functions for serde
fn default_build_jobs() -> usize {
    0 // 0 = auto-detect
}

fn default_timeout_seconds() -> u64 {
    3600 // 1 hour
}

fn default_build_root() -> PathBuf {
    PathBuf::from("/opt/pm/build")
}

fn default_cleanup_on_success() -> bool {
    true
}

fn default_strict_mode() -> bool {
    true
}

fn default_isolation_level() -> String {
    "default".to_string()
}

fn default_allow_network() -> bool {
    false
}

fn default_sbom_enabled() -> bool {
    true
}

fn default_sbom_format() -> String {
    "spdx-json".to_string()
}

fn default_include_build_info() -> bool {
    true
}

fn default_include_dependencies() -> bool {
    true
}

fn default_signing_enabled() -> bool {
    false
}

fn default_enable_hardened_runtime() -> bool {
    true
}

fn default_allowed_env_vars() -> Vec<String> {
    vec![
        "PATH".to_string(),
        "HOME".to_string(),
        "USER".to_string(),
        "SHELL".to_string(),
        "TERM".to_string(),
        "LANG".to_string(),
        "LC_ALL".to_string(),
        "CC".to_string(),
        "CXX".to_string(),
        "CFLAGS".to_string(),
        "CXXFLAGS".to_string(),
        "LDFLAGS".to_string(),
        "PKG_CONFIG_PATH".to_string(),
    ]
}

fn default_allowed_read_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr"),
        PathBuf::from("/opt/pm"),
        PathBuf::from("/System"),
        PathBuf::from("/Library"),
    ]
}

fn default_allowed_write_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/pm/build"),
        PathBuf::from("/opt/pm/store"),
        PathBuf::from("/tmp"),
    ]
}

fn default_cache_enabled() -> bool {
    true
}

fn default_cache_type() -> String {
    "ccache".to_string()
}

fn default_cache_dir() -> Option<PathBuf> {
    None
}

fn default_cache_size_mb() -> u64 {
    5000 // 5GB
}

fn default_distributed_cache() -> bool {
    false
}

fn default_parallel_builds() -> bool {
    true
}

fn default_out_of_source() -> bool {
    true
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
