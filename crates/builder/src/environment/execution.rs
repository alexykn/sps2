//! Command execution in isolated environment

use super::{core::BuildEnvironment, types::BuildCommandResult};
use sps2_errors::{BuildError, Error};
use sps2_events::{AppEvent, BuildDiagnostic, BuildEvent, EventEmitter, LogStream};
use sps2_platform::{PlatformContext, PlatformManager};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

impl BuildEnvironment {
    /// Convert command arguments to strings (no placeholder replacement needed)
    fn convert_args_to_strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    /// Get environment variables for execution (no placeholder replacement needed)
    fn get_execution_env(&self) -> HashMap<String, String> {
        self.env_vars.clone()
    }

    /// Execute a command in the build environment using the environment stored on the struct.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to spawn or exits with non-zero status.
    pub async fn execute_command(
        &self,
        program: &str,
        args: &[&str],
        working_dir: Option<&Path>,
    ) -> Result<BuildCommandResult, Error> {
        // Delegate to unified executor with the current environment and strict failure handling
        let env = self.get_execution_env();
        self.execute_command_with_env(program, args, working_dir, &env, false)
            .await
    }

    /// Execute a command with an explicit environment and optional allow-failure behavior.
    /// If `allow_failure` is true, this returns Ok(BuildCommandResult) even on non-zero exit codes.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to spawn. When `allow_failure` is false,
    /// returns an error for non-zero exit status as well.
    pub async fn execute_command_with_env(
        &self,
        program: &str,
        args: &[&str],
        working_dir: Option<&Path>,
        env: &HashMap<String, String>,
        allow_failure: bool,
    ) -> Result<BuildCommandResult, Error> {
        // Use platform abstraction for process execution
        let platform = PlatformManager::instance().platform();
        let context = PlatformContext::new(self.context.event_sender.clone());

        let mut cmd = platform.process().create_command(program);

        // Replace placeholders in command arguments
        let converted_args = Self::convert_args_to_strings(args);
        cmd.args(&converted_args);

        // Apply explicit environment
        cmd.envs(env);

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(&self.build_prefix);
        }

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

        let stdout_text = stdout_lines.join("\n");
        let stderr_text = stderr_lines.join("\n");

        if !stdout_text.is_empty() {
            let session_id = self.context.session_id();
            let command_id = Uuid::new_v4().to_string();
            self.emit(AppEvent::Build(BuildEvent::Diagnostic(
                BuildDiagnostic::LogChunk {
                    session_id: session_id.clone(),
                    command_id: Some(command_id.clone()),
                    stream: LogStream::Stdout,
                    text: stdout_text.clone(),
                },
            )));

            if !stderr_text.is_empty() {
                self.emit(AppEvent::Build(BuildEvent::Diagnostic(
                    BuildDiagnostic::LogChunk {
                        session_id,
                        command_id: Some(command_id),
                        stream: LogStream::Stderr,
                        text: stderr_text.clone(),
                    },
                )));
            }
        } else if !stderr_text.is_empty() {
            self.emit(AppEvent::Build(BuildEvent::Diagnostic(
                BuildDiagnostic::LogChunk {
                    session_id: self.context.session_id(),
                    command_id: Some(Uuid::new_v4().to_string()),
                    stream: LogStream::Stderr,
                    text: stderr_text.clone(),
                },
            )));
        }

        let result = BuildCommandResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: stdout_text,
            stderr: stderr_text,
        };

        if !result.success && !allow_failure {
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
        let libtool_candidate =
            std::path::Path::new(sps2_config::fixed_paths::BIN_DIR).join("libtool");
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
