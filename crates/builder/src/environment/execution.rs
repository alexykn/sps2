//! Command execution in isolated environment

use super::{core::BuildEnvironment, types::BuildCommandResult};
use crate::BuildContext;
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

impl BuildEnvironment {
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
        cmd.args(args);
        cmd.envs(&self.env_vars);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(&self.build_prefix);
        }

        self.send_event(Event::BuildStepStarted {
            step: format!("{program} {}", args.join(" ")),
            package: self.context.name.clone(),
        });

        // Send command info event to show what's running
        self.send_event(Event::DebugLog {
            message: format!("Executing: {program} {}", args.join(" ")),
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

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut stdout_lines = Vec::new();
        let mut stderr_lines = Vec::new();

        // Read output in real-time and print directly to stdout/stderr
        loop {
            tokio::select! {
                line = stdout_reader.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            // Send build output via events
                            Self::send_build_output(&self.context, &line, false);
                            stdout_lines.push(line);
                        }
                        Ok(None) => break,
                        Err(e) => {
                            return Err(BuildError::CompileFailed {
                                message: format!("Failed to read stdout: {e}"),
                            }.into());
                        }
                    }
                }
                line = stderr_reader.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            // Send build errors via events
                            Self::send_build_output(&self.context, &line, true);
                            stderr_lines.push(line);
                        }
                        Ok(None) => {},
                        Err(e) => {
                            return Err(BuildError::CompileFailed {
                                message: format!("Failed to read stderr: {e}"),
                            }.into());
                        }
                    }
                }
            }
        }

        let status = child.wait().await.map_err(|e| BuildError::CompileFailed {
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

        Ok(result)
    }

    /// Send build output via events instead of direct printing
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
}
