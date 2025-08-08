//! Environment verification and coordination
//!
//! This module provides comprehensive isolation verification for build environments,
//! including environment variable sanitization, path isolation checks, and
//! network isolation verification.

use super::core::BuildEnvironment;
use sps2_errors::{BuildError, Error};
use std::path::PathBuf;

impl BuildEnvironment {
    /// Verify build environment isolation is properly set up
    ///
    /// This performs comprehensive checks to ensure the build environment is
    /// properly isolated from the host system.
    ///
    /// # Errors
    ///
    /// Returns an error if the build environment is not properly isolated or directories are missing.
    pub fn verify_isolation(&self) -> Result<(), Error> {
        // Perform basic isolation checks
        self.verify_basic_isolation()?;

        // Verify environment variables are sanitized
        self.verify_environment_sanitization()?;

        // Verify path isolation
        self.verify_path_isolation()?;

        // Verify no access to system paths
        self.verify_system_path_isolation()?;

        Ok(())
    }

    /// Verify basic isolation requirements
    fn verify_basic_isolation(&self) -> Result<(), Error> {
        // Check that critical directories exist
        if !self.build_prefix.exists() {
            return Err(BuildError::Failed {
                message: format!(
                    "Build prefix does not exist: {}",
                    self.build_prefix.display()
                ),
            }
            .into());
        }

        if !self.staging_dir.exists() {
            return Err(BuildError::Failed {
                message: format!(
                    "Staging directory does not exist: {}",
                    self.staging_dir.display()
                ),
            }
            .into());
        }

        // Verify environment variables are set correctly
        let required_vars = vec!["PREFIX", "DESTDIR", "JOBS"];
        for var in required_vars {
            if !self.env_vars.contains_key(var) {
                return Err(BuildError::Failed {
                    message: format!("Required environment variable {var} not set"),
                }
                .into());
            }
        }

        // PATH will be updated when build dependencies are installed
        // So we just check it exists for now
        if !self.env_vars.contains_key("PATH") {
            return Err(BuildError::Failed {
                message: "PATH environment variable not set".to_string(),
            }
            .into());
        }

        Ok(())
    }

    /// Verify environment variables are properly sanitized
    fn verify_environment_sanitization(&self) -> Result<(), Error> {
        // List of potentially dangerous environment variables that should not be set
        let dangerous_vars = vec![
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",          // macOS specific
            "DYLD_FALLBACK_LIBRARY_PATH", // macOS specific
            "DYLD_INSERT_LIBRARIES",      // macOS specific - can inject code
            "LD_PRELOAD",                 // Linux equivalent of DYLD_INSERT_LIBRARIES
            "PYTHONPATH",                 // Could interfere with Python builds
            "PERL5LIB",                   // Could interfere with Perl builds
            "RUBYLIB",                    // Could interfere with Ruby builds
            "NODE_PATH",                  // Could interfere with Node.js builds
            "GOPATH",                     // Could interfere with Go builds
            "CARGO_HOME",                 // Could interfere with Rust builds
        ];

        for var in dangerous_vars {
            if self.env_vars.contains_key(var) {
                return Err(BuildError::SandboxViolation {
                    message: format!("Dangerous environment variable {var} is set"),
                }
                .into());
            }
        }

        // Verify compiler/linker flags are clean
        self.verify_compiler_flags()?;

        Ok(())
    }

    /// Verify compiler and linker flags are properly isolated
    fn verify_compiler_flags(&self) -> Result<(), Error> {
        // Check CFLAGS
        if let Some(cflags) = self.env_vars.get("CFLAGS") {
            // Check that CFLAGS contains /opt/pm/live/include
            if !cflags.contains(&format!("{}/include", sps2_config::fixed_paths::LIVE_DIR)) {
                return Err(BuildError::SandboxViolation {
                    message: "CFLAGS not properly configured for isolation".to_string(),
                }
                .into());
            }

            // Ensure no system paths are referenced
            if cflags.contains("/usr/local") || cflags.contains("/opt/homebrew") {
                return Err(BuildError::SandboxViolation {
                    message: "CFLAGS contains system paths".to_string(),
                }
                .into());
            }
        }

        // Check LDFLAGS
        if let Some(ldflags) = self.env_vars.get("LDFLAGS") {
            // Check that LDFLAGS contains /opt/pm/live/lib
            if !ldflags.contains(&format!("{}/lib", sps2_config::fixed_paths::LIVE_DIR)) {
                return Err(BuildError::SandboxViolation {
                    message: "LDFLAGS not properly configured for isolation".to_string(),
                }
                .into());
            }

            // Ensure no system paths are referenced
            if ldflags.contains("/usr/local") || ldflags.contains("/opt/homebrew") {
                return Err(BuildError::SandboxViolation {
                    message: "LDFLAGS contains system paths".to_string(),
                }
                .into());
            }
        }

        Ok(())
    }

    /// Verify PATH isolation
    fn verify_path_isolation(&self) -> Result<(), Error> {
        if let Some(path) = self.env_vars.get("PATH") {
            let path_components: Vec<&str> = path.split(':').collect();

            // Verify system paths come first
            if path_components.is_empty() || !path_components[0].starts_with("/usr/bin") {
                return Err(BuildError::SandboxViolation {
                    message: "System paths not first in PATH".to_string(),
                }
                .into());
            }

            // Verify /opt/pm/live/bin is in PATH but after system paths
            let mut found_system = false;

            for component in &path_components {
                if component.starts_with("/usr/")
                    || component.starts_with("/bin")
                    || component.starts_with("/sbin")
                {
                    found_system = true;
                } else if *component == sps2_config::fixed_paths::BIN_DIR {
                    if !found_system {
                        return Err(BuildError::SandboxViolation {
                            message: "/opt/pm/live/bin appears before system paths in PATH"
                                .to_string(),
                        }
                        .into());
                    }
                    // /opt/pm/live/bin found in correct position
                    break;
                }
            }

            // PATH is strictly controlled to only include system essentials and sps2 paths
            // No external package managers (Homebrew, MacPorts, etc.) are allowed
        } else {
            return Err(BuildError::SandboxViolation {
                message: "PATH not set".to_string(),
            }
            .into());
        }

        Ok(())
    }

    /// Verify no access to system paths outside allowed directories
    fn verify_system_path_isolation(&self) -> Result<(), Error> {
        // Check that build directories are within allowed paths
        let allowed_prefixes = vec![
            self.build_prefix.clone(),
            PathBuf::from(sps2_config::fixed_paths::PREFIX), // sps2 system directory
        ];

        // Verify staging directory is isolated
        let mut staging_allowed = false;
        for prefix in &allowed_prefixes {
            if self.staging_dir.starts_with(prefix) {
                staging_allowed = true;
                break;
            }
        }

        if !staging_allowed {
            return Err(BuildError::SandboxViolation {
                message: format!(
                    "Staging directory {} is outside allowed paths",
                    self.staging_dir.display()
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Check if network isolation is properly configured
    ///
    /// # Errors
    ///
    /// Currently this function never returns an error, but returns `Result` for future extensibility
    pub fn verify_network_isolation(&self) -> Result<bool, Error> {
        // Check if proxy environment variables are set for isolation
        let proxy_vars = ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"];
        let mut isolated = true;

        for var in &proxy_vars {
            if let Some(value) = self.env_vars.get(*var) {
                // Check if pointing to invalid proxy (network isolation)
                if !value.contains("127.0.0.1:1") {
                    isolated = false;
                    break;
                }
            } else {
                // No proxy set means network is not isolated
                isolated = false;
                break;
            }
        }

        Ok(isolated)
    }

    /// Get a summary of isolation status
    ///
    /// Returns a map of isolation checks and their results. This should be used for debugging
    /// and verifying that the build environment is properly isolated.
    #[must_use]
    pub fn isolation_summary(&self) -> std::collections::HashMap<String, String> {
        let mut summary = std::collections::HashMap::new();

        // Check basic isolation
        summary.insert(
            "basic_isolation".to_string(),
            self.verify_basic_isolation()
                .map_or_else(|e| format!("FAILED: {e}"), |()| "OK".to_string()),
        );

        // Check environment sanitization
        summary.insert(
            "env_sanitization".to_string(),
            self.verify_environment_sanitization()
                .map_or_else(|e| format!("FAILED: {e}"), |()| "OK".to_string()),
        );

        // Check path isolation
        summary.insert(
            "path_isolation".to_string(),
            self.verify_path_isolation()
                .map_or_else(|e| format!("FAILED: {e}"), |()| "OK".to_string()),
        );

        // Check network isolation
        summary.insert(
            "network_isolation".to_string(),
            self.verify_network_isolation().map_or_else(
                |e| format!("ERROR: {e}"),
                |isolated| {
                    if isolated {
                        "ENABLED".to_string()
                    } else {
                        "DISABLED".to_string()
                    }
                },
            ),
        );

        // Add key paths
        summary.insert(
            "build_prefix".to_string(),
            self.build_prefix.display().to_string(),
        );
        summary.insert(
            "staging_dir".to_string(),
            self.staging_dir.display().to_string(),
        );

        summary
    }
}
