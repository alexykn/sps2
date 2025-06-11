// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Hermetic build environment isolation
//!
//! This module provides comprehensive hermetic isolation features for build environments,
//! ensuring builds are reproducible and isolated from the host system.

use super::core::BuildEnvironment;
use sps2_errors::{BuildError, Error};
use sps2_events::{Event, EventSender};
use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Hermetic isolation configuration
#[derive(Debug, Clone)]
pub struct HermeticConfig {
    /// Environment variables to preserve (whitelist)
    pub allowed_env_vars: HashSet<String>,
    /// Paths that should be accessible read-only
    pub allowed_read_paths: Vec<PathBuf>,
    /// Paths that should be accessible read-write
    pub allowed_write_paths: Vec<PathBuf>,
    /// Whether to allow network access
    pub allow_network: bool,
    /// Temporary home directory path
    pub temp_home: Option<PathBuf>,
    /// Whether to create minimal device nodes
    pub create_devices: bool,
}

impl Default for HermeticConfig {
    fn default() -> Self {
        let mut allowed_env_vars = HashSet::new();
        // Minimal set of environment variables needed for builds
        allowed_env_vars.insert("PATH".to_string());
        allowed_env_vars.insert("HOME".to_string());
        allowed_env_vars.insert("TMPDIR".to_string());
        allowed_env_vars.insert("TEMP".to_string());
        allowed_env_vars.insert("TMP".to_string());
        allowed_env_vars.insert("USER".to_string());
        allowed_env_vars.insert("SHELL".to_string());
        allowed_env_vars.insert("TERM".to_string());

        Self {
            allowed_env_vars,
            allowed_read_paths: vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/usr/lib"),
                PathBuf::from("/usr/include"),
                PathBuf::from("/System"), // macOS system libraries
                PathBuf::from("/Library/Developer/CommandLineTools"), // Xcode tools
            ],
            allowed_write_paths: vec![],
            allow_network: false,
            temp_home: None,
            create_devices: false, // Not typically needed on macOS
        }
    }
}

impl BuildEnvironment {
    /// Apply hermetic isolation to the build environment
    ///
    /// # Errors
    ///
    /// Returns an error if isolation setup fails.
    pub async fn apply_hermetic_isolation(
        &mut self,
        config: &HermeticConfig,
        event_sender: Option<&EventSender>,
    ) -> Result<(), Error> {
        // Send event for isolation start
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::BuildStepStarted {
                package: self.context.name.clone(),
                step: "hermetic_isolation".to_string(),
            });
        }

        // Clear environment variables
        self.clear_environment_vars(config)?;

        // Setup temporary home directory
        let temp_home = self.setup_temp_home(config).await?;
        self.env_vars
            .insert("HOME".to_string(), temp_home.display().to_string());

        // Setup private temporary directory
        let private_tmp = self.setup_private_tmp().await?;
        self.env_vars
            .insert("TMPDIR".to_string(), private_tmp.display().to_string());
        self.env_vars
            .insert("TEMP".to_string(), private_tmp.display().to_string());
        self.env_vars
            .insert("TMP".to_string(), private_tmp.display().to_string());

        // Apply network isolation if configured
        if !config.allow_network {
            self.apply_network_isolation()?;
        }

        // Setup minimal device nodes if needed (mostly a no-op on macOS)
        if config.create_devices {
            self.setup_minimal_devices().await?;
        }

        // Send completion event
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::BuildStepCompleted {
                package: self.context.name.clone(),
                step: "hermetic_isolation".to_string(),
            });
        }

        Ok(())
    }

    /// Clear all environment variables except those whitelisted
    fn clear_environment_vars(&mut self, config: &HermeticConfig) -> Result<(), Error> {
        // Get current environment
        let current_env: HashMap<String, String> = std::env::vars().collect();

        // Start with a clean slate for the build environment
        self.env_vars.clear();

        // Only copy over whitelisted variables
        for (key, value) in current_env {
            if config.allowed_env_vars.contains(&key) {
                self.env_vars.insert(key, value);
            }
        }

        // Ensure critical build variables are set
        self.setup_clean_build_environment();

        Ok(())
    }

    /// Setup clean build environment variables
    fn setup_clean_build_environment(&mut self) {
        // Set clean PATH with only necessary directories
        let clean_path = [
            self.deps_prefix.join("bin").display().to_string(),
            "/usr/bin".to_string(),
            "/bin".to_string(),
            "/usr/sbin".to_string(),
            "/sbin".to_string(),
        ]
        .join(":");

        self.env_vars.insert("PATH".to_string(), clean_path);

        // Set build-specific variables
        self.env_vars
            .insert("PREFIX".to_string(), self.staging_dir.display().to_string());
        self.env_vars.insert(
            "DESTDIR".to_string(),
            self.staging_dir.display().to_string(),
        );
        self.env_vars
            .insert("JOBS".to_string(), Self::cpu_count().to_string());

        // Clean compiler/linker flags
        self.env_vars.insert(
            "CFLAGS".to_string(),
            format!("-I{}/include", self.deps_prefix.display()),
        );
        self.env_vars.insert(
            "CXXFLAGS".to_string(),
            format!("-I{}/include", self.deps_prefix.display()),
        );
        self.env_vars.insert(
            "LDFLAGS".to_string(),
            format!("-L{}/lib", self.deps_prefix.display()),
        );

        // Remove potentially harmful variables
        self.env_vars.remove("LD_LIBRARY_PATH");
        self.env_vars.remove("DYLD_LIBRARY_PATH"); // macOS specific
        self.env_vars.remove("DYLD_FALLBACK_LIBRARY_PATH"); // macOS specific
        self.env_vars.remove("PKG_CONFIG_PATH"); // Will be set when build deps are installed

        // Set locale to ensure consistent behavior
        self.env_vars
            .insert("LANG".to_string(), "C.UTF-8".to_string());
        self.env_vars.insert("LC_ALL".to_string(), "C".to_string());
    }

    /// Setup temporary home directory
    async fn setup_temp_home(&self, config: &HermeticConfig) -> Result<PathBuf, Error> {
        let temp_home = if let Some(ref home) = config.temp_home {
            home.clone()
        } else {
            self.build_prefix.join("home")
        };

        // Create the directory
        tokio::fs::create_dir_all(&temp_home)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to create temp home: {e}"),
            })?;

        // Set restrictive permissions
        let metadata = tokio::fs::metadata(&temp_home)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to get temp home metadata: {e}"),
            })?;

        let mut perms = metadata.permissions();
        perms.set_mode(0o700); // Owner read/write/execute only

        tokio::fs::set_permissions(&temp_home, perms)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to set temp home permissions: {e}"),
            })?;

        // Create minimal dot files to prevent tools from accessing real home
        self.create_minimal_dotfiles(&temp_home).await?;

        Ok(temp_home)
    }

    /// Create minimal dotfiles in temp home
    async fn create_minimal_dotfiles(&self, temp_home: &Path) -> Result<(), Error> {
        // Create empty .bashrc to prevent loading user's bashrc
        let bashrc = temp_home.join(".bashrc");
        tokio::fs::write(&bashrc, "# Hermetic build environment\n")
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to create .bashrc: {e}"),
            })?;

        // Create minimal .gitconfig to prevent git from accessing user config
        let gitconfig = temp_home.join(".gitconfig");
        let git_content = "[user]\n    name = sps2-builder\n    email = builder@sps2.local\n";
        tokio::fs::write(&gitconfig, git_content)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to create .gitconfig: {e}"),
            })?;

        // Create .config directory for tools that use XDG config
        let config_dir = temp_home.join(".config");
        tokio::fs::create_dir_all(&config_dir)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to create .config: {e}"),
            })?;

        Ok(())
    }

    /// Setup private temporary directory
    async fn setup_private_tmp(&self) -> Result<PathBuf, Error> {
        let private_tmp = self.build_prefix.join("tmp");

        // Create the directory
        tokio::fs::create_dir_all(&private_tmp)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to create private tmp: {e}"),
            })?;

        // Set sticky bit and appropriate permissions
        let metadata = tokio::fs::metadata(&private_tmp)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to get private tmp metadata: {e}"),
            })?;

        let mut perms = metadata.permissions();
        perms.set_mode(0o1777); // Sticky bit + world writable

        tokio::fs::set_permissions(&private_tmp, perms)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to set private tmp permissions: {e}"),
            })?;

        Ok(private_tmp)
    }

    /// Apply network isolation
    pub fn apply_network_isolation(&mut self) -> Result<(), Error> {
        // Set environment variables to disable network access
        self.env_vars
            .insert("http_proxy".to_string(), "http://127.0.0.1:1".to_string());
        self.env_vars
            .insert("https_proxy".to_string(), "http://127.0.0.1:1".to_string());
        self.env_vars
            .insert("ftp_proxy".to_string(), "http://127.0.0.1:1".to_string());
        self.env_vars
            .insert("all_proxy".to_string(), "http://127.0.0.1:1".to_string());
        self.env_vars
            .insert("HTTP_PROXY".to_string(), "http://127.0.0.1:1".to_string());
        self.env_vars
            .insert("HTTPS_PROXY".to_string(), "http://127.0.0.1:1".to_string());
        self.env_vars
            .insert("FTP_PROXY".to_string(), "http://127.0.0.1:1".to_string());
        self.env_vars
            .insert("ALL_PROXY".to_string(), "http://127.0.0.1:1".to_string());

        // Set no_proxy for localhost
        self.env_vars.insert(
            "no_proxy".to_string(),
            "localhost,127.0.0.1,::1".to_string(),
        );
        self.env_vars.insert(
            "NO_PROXY".to_string(),
            "localhost,127.0.0.1,::1".to_string(),
        );

        Ok(())
    }

    /// Setup minimal device nodes (mostly no-op on macOS)
    async fn setup_minimal_devices(&self) -> Result<(), Error> {
        // On macOS, we don't need to create device nodes as they're managed by the kernel
        // This is kept for API compatibility and future expansion
        Ok(())
    }

    /// Verify hermetic isolation is properly applied
    pub fn verify_hermetic_isolation(&self, config: &HermeticConfig) -> Result<(), Error> {
        // Check that only allowed environment variables are set
        for key in self.env_vars.keys() {
            // Build-specific variables are always allowed
            let build_vars = [
                "PREFIX",
                "DESTDIR",
                "JOBS",
                "CFLAGS",
                "CXXFLAGS",
                "LDFLAGS",
                "PKG_CONFIG_PATH",
                "LANG",
                "LC_ALL",
            ];

            if !config.allowed_env_vars.contains(key) && !build_vars.contains(&key.as_str()) {
                return Err(BuildError::SandboxViolation {
                    message: format!("Unexpected environment variable: {key}"),
                }
                .into());
            }
        }

        // Verify HOME is set to temp location
        if let Some(home) = self.env_vars.get("HOME") {
            let home_path = Path::new(home);
            if !home_path.starts_with(&self.build_prefix) {
                return Err(BuildError::SandboxViolation {
                    message: "HOME not pointing to isolated directory".to_string(),
                }
                .into());
            }
        } else {
            return Err(BuildError::SandboxViolation {
                message: "HOME environment variable not set".to_string(),
            }
            .into());
        }

        // Verify TMPDIR is set to private location
        if let Some(tmpdir) = self.env_vars.get("TMPDIR") {
            let tmp_path = Path::new(tmpdir);
            if !tmp_path.starts_with(&self.build_prefix) {
                return Err(BuildError::SandboxViolation {
                    message: "TMPDIR not pointing to isolated directory".to_string(),
                }
                .into());
            }
        } else {
            return Err(BuildError::SandboxViolation {
                message: "TMPDIR environment variable not set".to_string(),
            }
            .into());
        }

        // Verify network isolation if configured
        if !config.allow_network {
            let proxy_vars = ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"];
            for var in &proxy_vars {
                if let Some(value) = self.env_vars.get(*var) {
                    if !value.contains("127.0.0.1:1") {
                        return Err(BuildError::SandboxViolation {
                            message: format!("Network isolation not properly configured: {var}"),
                        }
                        .into());
                    }
                } else {
                    return Err(BuildError::SandboxViolation {
                        message: format!("Network isolation variable not set: {var}"),
                    }
                    .into());
                }
            }
        }

        Ok(())
    }
}
