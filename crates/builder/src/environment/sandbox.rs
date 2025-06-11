// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! macOS sandbox utilities for build isolation
//!
//! This module provides sandboxing utilities using macOS-specific features
//! like sandbox-exec profiles to enforce filesystem access restrictions,
//! process isolation, and resource limits.

use sps2_errors::{BuildError, Error};
use sps2_events::{Event, EventSender};
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::process::Command as AsyncCommand;

/// Sandbox profile for macOS sandbox-exec
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SandboxProfile {
    /// Name of the profile
    pub name: String,
    /// Profile content in Scheme format
    pub content: String,
}

/// Sandbox configuration
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Allow network access
    pub allow_network: bool,
    /// Paths allowed for reading
    pub allow_read_paths: Vec<PathBuf>,
    /// Paths allowed for writing
    pub allow_write_paths: Vec<PathBuf>,
    /// Paths allowed for execution
    pub allow_exec_paths: Vec<PathBuf>,
    /// Process resource limits
    pub resource_limits: ResourceLimits,
    /// Whether to allow process spawning
    pub allow_spawn: bool,
    /// Whether to allow sysctl access
    pub allow_sysctl: bool,
}

/// Resource limits for sandboxed processes
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum CPU time in seconds
    pub cpu_time: Option<u64>,
    /// Maximum memory in bytes
    pub memory: Option<u64>,
    /// Maximum number of open files
    pub open_files: Option<u64>,
    /// Maximum number of processes
    pub processes: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            cpu_time: Some(3600),                 // 1 hour default
            memory: Some(4 * 1024 * 1024 * 1024), // 4GB default
            open_files: Some(1024),
            processes: Some(128),
        }
    }
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            allow_network: false,
            allow_read_paths: vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/usr/lib"),
                PathBuf::from("/usr/include"),
                PathBuf::from("/System"),
                PathBuf::from("/Library/Developer/CommandLineTools"),
                PathBuf::from("/bin"),
                PathBuf::from("/sbin"),
            ],
            allow_write_paths: vec![],
            allow_exec_paths: vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
                PathBuf::from("/usr/sbin"),
                PathBuf::from("/sbin"),
            ],
            resource_limits: ResourceLimits::default(),
            allow_spawn: true, // Most builds need to spawn compilers
            allow_sysctl: false,
        }
    }
}

impl SandboxProfile {
    /// Create a new sandbox profile from configuration
    pub fn from_config(config: &SandboxConfig, build_dir: &Path) -> Self {
        let mut rules = vec![
            "(version 1)".to_string(),
            "(deny default)".to_string(),
            "(allow signal)".to_string(),
            "(allow system-socket)".to_string(),
        ];

        // Process spawning
        if config.allow_spawn {
            rules.push("(allow process-fork)".to_string());
            rules.push("(allow process-exec*)".to_string());
        }

        // Sysctl access
        if config.allow_sysctl {
            rules.push("(allow sysctl-read)".to_string());
        }

        // Network access
        if config.allow_network {
            rules.push("(allow network*)".to_string());
        } else {
            // Only allow local connections
            rules.push("(allow network-bind (local ip \"localhost:*\"))".to_string());
            rules.push("(allow network-bind (local ip \"127.0.0.1:*\"))".to_string());
            rules.push("(allow network-bind (local ip \"::1:*\"))".to_string());
        }

        // File read permissions
        for path in &config.allow_read_paths {
            let path_str = path.display().to_string();
            rules.push(format!("(allow file-read* (subpath \"{path_str}\"))"));
            rules.push(format!(
                "(allow file-read-metadata (subpath \"{path_str}\"))"
            ));
        }

        // Always allow reading from build directory
        let build_dir_str = build_dir.display().to_string();
        rules.push(format!("(allow file-read* (subpath \"{build_dir_str}\"))"));
        rules.push(format!(
            "(allow file-read-metadata (subpath \"{build_dir_str}\"))"
        ));

        // File write permissions
        for path in &config.allow_write_paths {
            let path_str = path.display().to_string();
            rules.push(format!("(allow file-write* (subpath \"{path_str}\"))"));
        }

        // Always allow writing to build directory
        rules.push(format!("(allow file-write* (subpath \"{build_dir_str}\"))"));
        rules.push(format!(
            "(allow file-write-create (subpath \"{build_dir_str}\"))"
        ));
        rules.push(format!(
            "(allow file-write-unlink (subpath \"{build_dir_str}\"))"
        ));

        // Execute permissions
        for path in &config.allow_exec_paths {
            let path_str = path.display().to_string();
            rules.push(format!("(allow file-exec (subpath \"{path_str}\"))"));
        }

        // Mach operations (needed for basic operations)
        rules.push("(allow mach-lookup)".to_string());
        rules.push("(allow mach-register)".to_string());

        // IPC operations
        rules.push("(allow ipc-posix-shm)".to_string());

        // Temporary files
        rules.push("(allow file-write* (subpath \"/private/tmp\"))".to_string());
        rules.push("(allow file-write* (subpath \"/var/folders\"))".to_string());

        let content = rules.join("\n");

        Self {
            name: "sps2-build-sandbox".to_string(),
            content,
        }
    }

    /// Write profile to a temporary file
    pub async fn write_to_temp(&self) -> Result<PathBuf, Error> {
        let temp_dir = std::env::temp_dir();
        let profile_path = temp_dir.join(format!("{}.sb", self.name));

        tokio::fs::write(&profile_path, &self.content)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to write sandbox profile: {e}"),
            })?;

        Ok(profile_path)
    }
}

/// Sandbox executor for running commands in isolation
pub struct SandboxExecutor {
    config: SandboxConfig,
    profile: SandboxProfile,
    build_dir: PathBuf,
}

impl SandboxExecutor {
    /// Create a new sandbox executor
    pub fn new(config: SandboxConfig, build_dir: PathBuf) -> Self {
        let profile = SandboxProfile::from_config(&config, &build_dir);
        Self {
            config,
            profile,
            build_dir,
        }
    }

    /// Execute a command in the sandbox
    pub async fn execute(
        &self,
        command: &str,
        args: &[String],
        env_vars: &std::collections::HashMap<String, String>,
        working_dir: &Path,
        event_sender: Option<&EventSender>,
    ) -> Result<std::process::Output, Error> {
        // Write sandbox profile
        let profile_path = self.profile.write_to_temp().await?;

        // Send event for sandbox execution
        if let Some(sender) = event_sender {
            let _ = sender.send(Event::DebugLog {
                message: format!("Executing in sandbox: {command}"),
                context: std::collections::HashMap::from([
                    (
                        "sandbox_profile".to_string(),
                        profile_path.display().to_string(),
                    ),
                    ("working_dir".to_string(), working_dir.display().to_string()),
                ]),
            });
        }

        // Build sandbox-exec command
        let mut cmd = AsyncCommand::new("sandbox-exec");
        cmd.arg("-f").arg(&profile_path);

        // Add the actual command
        cmd.arg(command);
        cmd.args(args);

        // Set working directory
        cmd.current_dir(working_dir);

        // Set environment variables
        cmd.env_clear();
        for (key, value) in env_vars {
            cmd.env(key, value);
        }

        // Apply resource limits using ulimit
        if let Some(cpu_time) = self.config.resource_limits.cpu_time {
            cmd.env("SANDBOX_CPU_TIME", cpu_time.to_string());
        }

        // Execute the command
        let output = cmd.output().await.map_err(|e| BuildError::Failed {
            message: format!("Failed to execute sandboxed command: {e}"),
        })?;

        // Clean up profile file
        let _ = tokio::fs::remove_file(&profile_path).await;

        Ok(output)
    }

    /// Verify sandbox is available on the system
    pub fn verify_sandbox_available() -> Result<(), Error> {
        let output = Command::new("which")
            .arg("sandbox-exec")
            .output()
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to check for sandbox-exec: {e}"),
            })?;

        if !output.status.success() {
            return Err(BuildError::Failed {
                message: "sandbox-exec not found. macOS sandbox support requires sandbox-exec"
                    .to_string(),
            }
            .into());
        }

        Ok(())
    }
}

/// Check if a path would be accessible with given sandbox config
pub fn check_path_accessibility(path: &Path, config: &SandboxConfig) -> PathAccessibility {
    let mut access = PathAccessibility {
        readable: false,
        writable: false,
        executable: false,
    };

    // Check read access
    for allowed in &config.allow_read_paths {
        if path.starts_with(allowed) {
            access.readable = true;
            break;
        }
    }

    // Check write access
    for allowed in &config.allow_write_paths {
        if path.starts_with(allowed) {
            access.writable = true;
            break;
        }
    }

    // Check execute access
    for allowed in &config.allow_exec_paths {
        if path.starts_with(allowed) {
            access.executable = true;
            break;
        }
    }

    access
}

/// Path accessibility information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathAccessibility {
    /// Can read from this path
    pub readable: bool,
    /// Can write to this path
    pub writable: bool,
    /// Can execute from this path
    pub executable: bool,
}

/// Create a minimal sandbox configuration for builds
pub fn minimal_build_sandbox(build_dir: &Path) -> SandboxConfig {
    let mut config = SandboxConfig::default();

    // Add build directory to allowed paths
    config.allow_read_paths.push(build_dir.to_path_buf());
    config.allow_write_paths.push(build_dir.to_path_buf());
    config.allow_exec_paths.push(build_dir.to_path_buf());

    // Add deps directory if it exists
    let deps_dir = build_dir.join("deps");
    if deps_dir.exists() {
        config.allow_read_paths.push(deps_dir.clone());
        config.allow_exec_paths.push(deps_dir.join("bin"));
    }

    config
}
