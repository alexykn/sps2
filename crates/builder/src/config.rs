#![deny(clippy::pedantic, unsafe_code)]
//! Builder configuration integration and utilities
//!
//! This module provides integration between the builder crate and the unified
//! configuration system in `sps2_config`. All configuration types are now
//! centralized in the config crate.

use sps2_config::builder::{
    BuildSettings, BuilderConfig, CacheSettings, CompressionSettings, EnvironmentSettings,
    PackagingSettings, PerformanceSettings, SbomSettings, SecuritySettings, ShellExpansionPolicy,
    SigningSettings, ValidationConfig, ValidationMode,
};
use sps2_resources::ResourceManager;
use std::sync::Arc;

/// Builder context configuration
///
/// This struct adapts the centralized `BuilderConfig` for use within
/// the builder crate, adding runtime-specific fields like `ResourceManager`.
#[derive(Clone, Debug)]
pub struct BuildConfig {
    /// Core builder configuration from config crate
    pub config: BuilderConfig,
    /// Resource manager for build operations
    pub resources: Arc<ResourceManager>,
    /// sps2 system configuration (for command validation)
    pub sps2_config: Option<sps2_config::Config>,
}

impl BuildConfig {
    /// Create a new `BuildConfig` from a `BuilderConfig`
    #[must_use]
    pub fn new(config: BuilderConfig) -> Self {
        Self {
            config,
            resources: Arc::new(ResourceManager::default()),
            sps2_config: None,
        }
    }

    /// Create a new `BuildConfig` with custom `ResourceManager`
    #[must_use]
    pub fn with_resources(config: BuilderConfig, resources: Arc<ResourceManager>) -> Self {
        Self {
            config,
            resources,
            sps2_config: None,
        }
    }

    /// Create a new `BuildConfig` with `sps2_config`
    #[must_use]
    pub fn with_sps2_config(mut self, sps2_config: sps2_config::Config) -> Self {
        self.sps2_config = Some(sps2_config);
        self
    }

    /// Get build settings
    #[must_use]
    pub fn build_settings(&self) -> &BuildSettings {
        &self.config.build
    }

    /// Get packaging settings
    #[must_use]
    pub fn packaging_settings(&self) -> &PackagingSettings {
        &self.config.packaging
    }

    /// Get environment settings
    #[must_use]
    pub fn environment_settings(&self) -> &EnvironmentSettings {
        &self.config.environment
    }

    /// Get performance settings
    #[must_use]
    pub fn performance_settings(&self) -> &PerformanceSettings {
        &self.config.performance
    }

    /// Get security settings
    #[must_use]
    pub fn security_settings(&self) -> &SecuritySettings {
        &self.config.security
    }

    /// Get SBOM configuration
    #[must_use]
    pub fn sbom_config(&self) -> &SbomSettings {
        &self.config.packaging.sbom
    }

    /// Get signing configuration
    #[must_use]
    pub fn signing_config(&self) -> &SigningSettings {
        &self.config.packaging.signing
    }

    /// Get compression configuration
    #[must_use]
    pub fn compression_config(&self) -> &CompressionSettings {
        &self.config.packaging.compression
    }

    /// Get cache configuration
    #[must_use]
    pub fn cache_config(&self) -> &CacheSettings {
        &self.config.performance.cache
    }

    /// Get validation configuration
    #[must_use]
    pub fn validation_config(&self) -> &ValidationConfig {
        &self.config.security.validation
    }

    /// Get default network access policy (can be overridden by recipe)
    #[must_use]
    pub fn default_allow_network(&self) -> bool {
        self.config.build.default_allow_network
    }

    /// Get default isolation level (can be overridden by recipe)
    #[must_use]
    pub fn default_isolation_level(&self) -> &str {
        &self.config.build.default_isolation_level
    }

    /// Get maximum build time in seconds
    #[must_use]
    pub fn max_build_time(&self) -> Option<u64> {
        Some(self.config.build.timeout_seconds)
    }

    /// Get number of parallel build jobs
    #[must_use]
    pub fn build_jobs(&self) -> Option<usize> {
        if self.config.build.build_jobs == 0 {
            None // Auto-detect
        } else {
            Some(self.config.build.build_jobs)
        }
    }

    /// Get build root directory
    #[must_use]
    pub fn build_root(&self) -> &std::path::Path {
        &self.config.build.build_root
    }

    /// Check if strict validation is enabled
    #[must_use]
    pub fn is_strict_validation(&self) -> bool {
        matches!(self.config.security.validation.mode, ValidationMode::Strict)
    }

    /// Check if shell expansion is allowed
    #[must_use]
    pub fn allow_shell_expansion(&self) -> bool {
        matches!(
            self.config.security.validation.shell_expansion,
            ShellExpansionPolicy::Enabled
        )
    }

    /// Check if a command is allowed
    #[must_use]
    pub fn is_command_allowed(&self, command: &str) -> bool {
        self.config.is_command_allowed(command)
    }

    /// Check if a shell pattern is allowed
    #[must_use]
    pub fn is_shell_pattern_allowed(&self, pattern: &str) -> bool {
        self.config.is_shell_pattern_allowed(pattern)
    }

    /// Get all allowed commands
    #[must_use]
    pub fn get_allowed_commands(&self) -> Vec<String> {
        self.config.get_allowed_commands()
    }

    /// Validate the configuration
    ///
    /// # Errors
    ///
    /// Returns an error if any configuration values are invalid
    pub fn validate(&self) -> Result<(), sps2_errors::Error> {
        self.config.validate()
    }

    /// Get access to `sps2_config` (for compatibility)
    #[must_use]
    pub fn sps2_config(&self) -> Option<&sps2_config::Config> {
        self.sps2_config.as_ref()
    }

    // Builder pattern methods for backward compatibility

    /// Create config with network access enabled (deprecated - network comes from recipe)
    #[must_use]
    pub fn with_network() -> Self {
        // Network access should come from recipe, not config
        Self::default()
    }

    /// Set SBOM configuration
    #[must_use]
    pub fn with_sbom_config(mut self, sbom_config: SbomSettings) -> Self {
        self.config.packaging.sbom = sbom_config;
        self
    }

    /// Set signing configuration
    #[must_use]
    pub fn with_signing_config(mut self, signing_config: SigningSettings) -> Self {
        self.config.packaging.signing = signing_config;
        self
    }

    /// Set build timeout
    #[must_use]
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.config.build.timeout_seconds = seconds;
        self
    }

    /// Set parallel build jobs
    #[must_use]
    pub fn with_jobs(mut self, jobs: usize) -> Self {
        self.config.build.build_jobs = jobs;
        self
    }

    /// Set compression configuration
    #[must_use]
    pub fn with_compression_config(mut self, compression_config: CompressionSettings) -> Self {
        self.config.packaging.compression = compression_config;
        self
    }

    /// Set compression level
    #[must_use]
    pub fn with_compression_level(mut self, level: String) -> Self {
        self.config.packaging.compression.level = level;
        self
    }

    /// Enable fast compression for development builds
    #[must_use]
    pub fn with_fast_compression() -> Self {
        let mut config = BuilderConfig::default();
        config.packaging.compression.level = "fast".to_string();
        Self::new(config)
    }

    /// Enable balanced compression (default)
    #[must_use]
    pub fn with_balanced_compression() -> Self {
        let mut config = BuilderConfig::default();
        config.packaging.compression.level = "balanced".to_string();
        Self::new(config)
    }

    /// Enable maximum compression for production builds
    #[must_use]
    pub fn with_maximum_compression() -> Self {
        let mut config = BuilderConfig::default();
        config.packaging.compression.level = "maximum".to_string();
        Self::new(config)
    }

    /// Enable custom compression level
    #[must_use]
    pub fn with_custom_compression(level: u8) -> Self {
        let mut config = BuilderConfig::default();
        config.packaging.compression.level = level.to_string();
        Self::new(config)
    }

    /// Set build root directory
    #[must_use]
    pub fn with_build_root(mut self, path: std::path::PathBuf) -> Self {
        self.config.build.build_root = path;
        self
    }

    /// Set isolation level (deprecated - isolation comes from recipe)
    #[must_use]
    pub fn with_isolation_level(self, _level: &str) -> Self {
        // Isolation level should come from recipe, not config
        self
    }
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self::new(BuilderConfig::default())
    }
}

impl From<BuilderConfig> for BuildConfig {
    fn from(config: BuilderConfig) -> Self {
        Self::new(config)
    }
}

// Re-export commonly used types for convenience
pub use sps2_config::builder::{
    BuildSystemSettings, CommandsConfig, ShellExpansionPolicy as ConfigShellExpansionPolicy,
    ValidationMode as ConfigValidationMode,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_config_creation() {
        let builder_config = BuilderConfig::default();
        let build_config = BuildConfig::new(builder_config);

        assert_eq!(build_config.max_build_time(), Some(3600));
        assert!(build_config.is_strict_validation());
        assert!(!build_config.allow_shell_expansion());
    }

    #[test]
    fn test_build_config_validation() {
        let build_config = BuildConfig::default();
        assert!(build_config.validate().is_ok());
    }
}
