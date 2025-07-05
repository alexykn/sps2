//! Command execution in isolated environment

use super::{core::BuildEnvironment, types::BuildCommandResult};
use crate::BuildContext;
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

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
        let mut cmd = Command::new(program);

        // Replace placeholders in command arguments
        let converted_args = Self::convert_args_to_strings(args);
        let arg_refs: Vec<&str> = converted_args.iter().map(String::as_str).collect();
        cmd.args(&arg_refs);

        // Get environment variables for execution
        let build_env_vars = self.get_execution_env();
        cmd.envs(&build_env_vars);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(&self.build_prefix);
        }

        self.send_event(Event::BuildStepStarted {
            step: format!("{program} {}", arg_refs.join(" ")),
            package: self.context.name.clone(),
        });

        // Send command info event to show what's running (with replaced paths)
        self.send_event(Event::DebugLog {
            message: format!("Executing: {program} {}", arg_refs.join(" ")),
            context: std::collections::HashMap::from([(
                "working_dir".to_string(),
                working_dir.map_or_else(
                    || self.build_prefix.display().to_string(),
                    |p| p.display().to_string(),
                ),
            )]),
        });

        let mut child = cmd.spawn().map_err(|e| BuildError::CompileFailed {
            message: format!("{program}: {e}"),
        })?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let (stdout_lines, stderr_lines) = self
            .handle_process_output(stdout, stderr, &mut child)
            .await?;

        // Add timeout for individual commands (default: 10 minutes)
        let status = tokio::time::timeout(
            std::time::Duration::from_secs(600), // 10 minutes timeout
            child.wait(),
        )
        .await
        .map_err(|_| BuildError::CompileFailed {
            message: format!(
                "Command '{program} {}' timed out after 10 minutes",
                args.join(" ")
            ),
        })?
        .map_err(|e| BuildError::CompileFailed {
            message: format!("Failed to wait for {program}: {e}"),
        })?;

        let result = BuildCommandResult {
            success: status.success(),
            exit_code: status.code(),
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

    /// Handle process output streams with timeout and real-time event emission
    async fn handle_process_output(
        &self,
        stdout: tokio::process::ChildStdout,
        stderr: tokio::process::ChildStderr,
        child: &mut tokio::process::Child,
    ) -> Result<(Vec<String>, Vec<String>), Error> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut stdout_lines = Vec::new();
        let mut stderr_lines = Vec::new();

        // Read output in real-time with timeout to prevent deadlock
        let mut stdout_closed = false;
        let mut stderr_closed = false;

        while !stdout_closed || !stderr_closed {
            tokio::select! {
                line = stdout_reader.next_line(), if !stdout_closed => {
                    match line {
                        Ok(Some(line)) => {
                            // Send build output via events
                            Self::send_build_output(&self.context, &line, false);
                            stdout_lines.push(line);
                        }
                        Ok(None) => stdout_closed = true,
                        Err(e) => {
                            return Err(BuildError::CompileFailed {
                                message: format!("Failed to read stdout: {e}"),
                            }.into());
                        }
                    }
                }
                line = stderr_reader.next_line(), if !stderr_closed => {
                    match line {
                        Ok(Some(line)) => {
                            // Send stderr as normal build output (not error) since many tools output progress to stderr
                            Self::send_build_output(&self.context, &line, false);
                            stderr_lines.push(line);
                        }
                        Ok(None) => stderr_closed = true,
                        Err(e) => {
                            return Err(BuildError::CompileFailed {
                                message: format!("Failed to read stderr: {e}"),
                            }.into());
                        }
                    }
                }
                // Add timeout to prevent hanging on output reading
                () = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                    // Check if child process is still alive
                    match child.try_wait() {
                        Ok(Some(_)) => {
                            // Process finished, break out of loop
                            break;
                        }
                        Ok(None) => {
                            // Process still running, continue reading
                        }
                        Err(e) => {
                            return Err(BuildError::CompileFailed {
                                message: format!("Failed to check process status: {e}"),
                            }.into());
                        }
                    }
                }
            }
        }

        Ok((stdout_lines, stderr_lines))
    }

    /// Send build output via events instead of direct printing
    /// Note: `is_error` should only be true for actual errors, not stderr output
    fn send_build_output(context: &BuildContext, line: &str, is_error: bool) {
        if let Some(sender) = &context.event_sender {
            let _ = sender.send(if is_error {
                Event::Error {
                    message: line.to_string(),
                    details: Some("Build stderr".to_string()),
                }
            } else {
                Event::BuildStepOutput {
                    package: context.name.clone(),
                    line: line.to_string(),
                }
            });
        }
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
        // Use GNU libtool from /opt/pm/live/bin if it exists, otherwise try system libtool
        let libtool_path = if std::path::Path::new("/opt/pm/live/bin/libtool").exists() {
            "/opt/pm/live/bin/libtool"
        } else {
            "libtool"
        };

        let mut cmd = Command::new(libtool_path);
        cmd.args(["--finish", dir]);
        cmd.envs(&self.env_vars);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.current_dir(&self.build_prefix);

        let output = cmd.output().await.map_err(|e| BuildError::CompileFailed {
            message: format!("Failed to run libtool --finish: {e}"),
        })?;

        Ok(BuildCommandResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }

    /// Handle libtool --finish requirements from command output
    async fn handle_libtool_finish(&self, result: &BuildCommandResult) -> Result<(), Error> {
        let libtool_dirs = Self::check_libtool_finish_needed(result);
        if !libtool_dirs.is_empty() {
            for dir in &libtool_dirs {
                self.send_event(Event::DebugLog {
                    message: format!("Running libtool --finish {dir}"),
                    context: std::collections::HashMap::new(),
                });

                // Run libtool --finish for this directory
                let finish_result = self.execute_libtool_finish(dir).await?;
                if !finish_result.success {
                    self.send_event(Event::Warning {
                        message: format!("libtool --finish {dir} failed"),
                        context: Some(finish_result.stderr),
                    });
                }
            }
        }
        Ok(())
    }
}
