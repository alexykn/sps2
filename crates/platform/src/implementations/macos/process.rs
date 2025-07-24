//! macOS process operations implementation
//! 
//! This module wraps the existing proven process execution patterns from the codebase
//! with the platform abstraction layer, adding event emission and proper error handling.

use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Instant;
use sps2_errors::{PlatformError, Error};
use sps2_events::{AppEvent, events::PlatformEvent};
use tokio::process::Command;

use crate::process::{ProcessOperations, PlatformCommand, CommandOutput};
use crate::core::PlatformContext;

/// macOS implementation of process operations
pub struct MacOSProcessOperations;

impl MacOSProcessOperations {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MacOSProcessOperations {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProcessOperations for MacOSProcessOperations {
    async fn execute_command(&self, ctx: &PlatformContext, cmd: PlatformCommand) -> Result<CommandOutput, Error> {
        let start = Instant::now();
        let command_str = cmd.program().to_string();
        let args_clone = cmd.get_args().to_vec();
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::ProcessExecutionStarted {
            command: command_str.clone(),
            args: args_clone.clone(),
            working_dir: cmd.get_current_dir().map(|p| p.to_string_lossy().to_string()),
        })).await;

        // Use the proven tokio Command execution pattern from the codebase
        let result: Result<CommandOutput, PlatformError> = async {
            let mut command = Command::new(cmd.program());
            command.args(cmd.get_args());
            
            if let Some(dir) = cmd.get_current_dir() {
                command.current_dir(dir);
            }

            let output = command.output().await.map_err(|e| {
                PlatformError::ProcessExecutionFailed {
                    command: cmd.program().to_string(),
                    message: e.to_string(),
                }
            })?;

            Ok(CommandOutput {
                status: output.status,
                stdout: output.stdout,
                stderr: output.stderr,
            })
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(output) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::ProcessExecutionCompleted {
                    command: command_str,
                    exit_code: output.status.code().unwrap_or(-1),
                    duration_ms: duration.as_millis() as u64,
                    stdout_bytes: output.stdout.len(),
                    stderr_bytes: output.stderr.len(),
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::ProcessExecutionFailed {
                    command: command_str,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result.map_err(Error::from)
    }
    
    fn create_command(&self, program: &str) -> PlatformCommand {
        PlatformCommand::new(program)
    }
    
    async fn which(&self, program: &str) -> Result<PathBuf, Error> {
        // Use the standard which command implementation
        let output = Command::new("which")
            .arg(program)
            .output()
            .await
            .map_err(|e| Error::from(PlatformError::ProcessExecutionFailed {
                command: "which".to_string(),
                message: e.to_string(),
            }))?;

        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(PathBuf::from(path_str))
        } else {
            Err(Error::from(PlatformError::CommandNotFound {
                command: program.to_string(),
            }))
        }
    }
}