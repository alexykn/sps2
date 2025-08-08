//! Command execution in isolated environment

use super::{core::BuildEnvironment, types::BuildCommandResult};
use sps2_errors::{BuildError, Error};
use sps2_events::{AppEvent, BuildEvent, EventEmitter};
use sps2_platform::{PlatformContext, PlatformManager};
use std::collections::HashMap;
use std::path::Path;

impl BuildEnvironment {
    /// Convert command arguments to strings (no placeholder replacement needed)
    fn convert_args_to_strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    /// Get environment variables for execution (no placeholder replacement needed)
    fn get_execution_env(&self) -> HashMap<String, String> {
        self.env_vars.clone()
    }

    /// Execute a command in the build environment
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute or exits with a non-zero status.
    ///
    /// # Panics
    ///
    /// Panics if stdout is not available when capturing command output.
    pub async fn execute_command(
        &self,
        program: &str,
        args: &[&str],
        working_dir: Option<&Path>,
    ) -> Result<BuildCommandResult, Error> {
        // Use platform abstraction for process execution
        let platform = PlatformManager::instance().platform();
        let context = PlatformContext::new(self.context.event_sender.clone());

        let mut cmd = platform.process().create_command(program);

        // Replace placeholders in command arguments
        let converted_args = Self::convert_args_to_strings(args);
        cmd.args(&converted_args);

        // Get environment variables for execution
        let build_env_vars = self.get_execution_env();
        cmd.envs(&build_env_vars);

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(&self.build_prefix);
        }

        self.emit(AppEvent::Build(BuildEvent::CommandStarted {
            session_id: format!("build-{}", self.context.name),
            package: self.context.name.clone(),
            command_id: format!("cmd-{}", std::process::id()),
            build_system: sps2_events::BuildSystem::Custom,
            command: format!("{program} {}", converted_args.join(" ")),
            working_dir: self.build_prefix.clone(),
            timeout: None,
        }));

        // Send command info event to show what's running (with replaced paths)
        self.emit_debug_with_context(
            format!("Executing: {program} {}", converted_args.join(" ")),
            std::collections::HashMap::from([(
                "working_dir".to_string(),
                working_dir.map_or_else(
                    || self.build_prefix.display().to_string(),
                    |p| p.display().to_string(),
                ),
            )]),
        );

        let output = platform
            .process()
            .execute_command(&context, cmd)
            .await
            .map_err(|e| BuildError::CompileFailed {
                message: format!("{program}: {e}"),
            })?;

        let stdout_lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(std::string::ToString::to_string)
            .collect();

        let stderr_lines: Vec<String> = String::from_utf8_lossy(&output.stderr)
            .lines()
            .map(std::string::ToString::to_string)
            .collect();

        let result = BuildCommandResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: stdout_lines.join("\n"),
            stderr: stderr_lines.join("\n"),
        };

        if !result.success {
            return Err(BuildError::CompileFailed {
                message: format!(
                    "{program} {} failed with exit code {:?}: {}",
                    args.join(" "),
                    result.exit_code,
                    result.stderr
                ),
            }
            .into());
        }

        // Handle any libtool --finish requirements
        self.handle_libtool_finish(&result).await?;

        Ok(result)
    }

    /// Check if libtool --finish needs to be run based on command output
    fn check_libtool_finish_needed(result: &BuildCommandResult) -> Vec<String> {
        use std::collections::HashSet;
        let mut dirs = HashSet::new();

        // Check both stdout and stderr for the libtool warning
        let combined_output = format!("{}\n{}", result.stdout, result.stderr);

        // Look for the pattern: "remember to run `libtool --finish /path/to/lib'"
        for line in combined_output.lines() {
            if line.contains("remember to run") && line.contains("libtool --finish") {
                // Extract the directory path from the message
                // Pattern: "warning: remember to run `libtool --finish /opt/pm/live/lib'"
                if let Some(start) = line.find("libtool --finish") {
                    let remainder = &line[start + "libtool --finish".len()..];
                    // Find the directory path (everything up to the closing quote or end of line)
                    let dir_end = remainder
                        .find('\'')
                        .or_else(|| remainder.find('"'))
                        .unwrap_or(remainder.len());
                    let dir_path = remainder[..dir_end].trim();
                    if !dir_path.is_empty() {
                        dirs.insert(dir_path.to_string());
                    }
                }
            }
        }

        dirs.into_iter().collect()
    }

    /// Execute libtool --finish for the given directory
    async fn execute_libtool_finish(&self, dir: &str) -> Result<BuildCommandResult, Error> {
        // Use GNU libtool from fixed bin dir if it exists, otherwise try system libtool
        let libtool_candidate = std::path::Path::new(sps2_config::fixed_paths::BIN_DIR)
            .join("libtool");
        let libtool_path = if libtool_candidate.exists() {
            libtool_candidate.display().to_string()
        } else {
            "libtool".to_string()
        };

        // Use platform abstraction for process execution
        let platform = PlatformManager::instance().platform();
        let context = PlatformContext::new(self.context.event_sender.clone());

        let mut cmd = platform.process().create_command(&libtool_path);
        cmd.args(["--finish", dir]);
        cmd.envs(&self.env_vars);
        cmd.current_dir(&self.build_prefix);

        let output = platform
            .process()
            .execute_command(&context, cmd)
            .await
            .map_err(|e| BuildError::CompileFailed {
                message: format!("Failed to run libtool --finish: {e}"),
            })?;

        let stdout_lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(std::string::ToString::to_string)
            .collect();

        let stderr_lines: Vec<String> = String::from_utf8_lossy(&output.stderr)
            .lines()
            .map(std::string::ToString::to_string)
            .collect();

        Ok(BuildCommandResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: stdout_lines.join("\n"),
            stderr: stderr_lines.join("\n"),
        })
    }

    /// Handle libtool --finish requirements from command output
    async fn handle_libtool_finish(&self, result: &BuildCommandResult) -> Result<(), Error> {
        let libtool_dirs = Self::check_libtool_finish_needed(result);
        if !libtool_dirs.is_empty() {
            for dir in &libtool_dirs {
                self.emit_debug(format!("Running libtool --finish {dir}"));

                // Run libtool --finish for this directory
                let finish_result = self.execute_libtool_finish(dir).await?;
                if !finish_result.success {
                    self.emit_warning_with_context(
                        format!("libtool --finish {dir} failed"),
                        finish_result.stderr,
                    );
                }
            }
        }
        Ok(())
    }
}
