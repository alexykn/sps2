//! macOS process operations implementation
//!
//! This module wraps the existing proven process execution patterns from the codebase
//! with the platform abstraction layer, adding event emission and proper error handling.

use async_trait::async_trait;
use sps2_errors::{Error, PlatformError};
use sps2_events::{
    events::{
        FailureContext, PlatformEvent, PlatformOperationContext, PlatformOperationKind,
        PlatformOperationMetrics, ProcessCommandDescriptor,
    },
    AppEvent,
};
use std::convert::TryFrom;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Command;

use crate::core::PlatformContext;
use crate::process::{CommandOutput, PlatformCommand, ProcessOperations};

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

fn process_context(descriptor: ProcessCommandDescriptor) -> PlatformOperationContext {
    PlatformOperationContext {
        kind: PlatformOperationKind::Process,
        operation: "execute_command".to_string(),
        target: None,
        source: None,
        command: Some(descriptor),
    }
}

fn process_metrics(duration: Duration, output: Option<&CommandOutput>) -> PlatformOperationMetrics {
    PlatformOperationMetrics {
        duration_ms: Some(duration_to_millis(duration)),
        exit_code: output.and_then(|o| o.status.code()),
        stdout_bytes: output.and_then(|o| u64::try_from(o.stdout.len()).ok()),
        stderr_bytes: output.and_then(|o| u64::try_from(o.stderr.len()).ok()),
        changes: None,
    }
}

fn duration_to_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

async fn emit_process_started(ctx: &PlatformContext, descriptor: &ProcessCommandDescriptor) {
    ctx.emit_event(AppEvent::Platform(PlatformEvent::OperationStarted {
        context: process_context(descriptor.clone()),
    }))
    .await;
}

async fn emit_process_completed(
    ctx: &PlatformContext,
    descriptor: &ProcessCommandDescriptor,
    output: &CommandOutput,
    duration: Duration,
) {
    ctx.emit_event(AppEvent::Platform(PlatformEvent::OperationCompleted {
        context: process_context(descriptor.clone()),
        metrics: Some(process_metrics(duration, Some(output))),
    }))
    .await;
}

async fn emit_process_failed(
    ctx: &PlatformContext,
    descriptor: &ProcessCommandDescriptor,
    error: &PlatformError,
    duration: Duration,
) {
    ctx.emit_event(AppEvent::Platform(PlatformEvent::OperationFailed {
        context: process_context(descriptor.clone()),
        failure: FailureContext::from_error(error),
        metrics: Some(process_metrics(duration, None)),
    }))
    .await;
}

#[async_trait]
impl ProcessOperations for MacOSProcessOperations {
    async fn execute_command(
        &self,
        ctx: &PlatformContext,
        cmd: PlatformCommand,
    ) -> Result<CommandOutput, Error> {
        let start = Instant::now();
        let command_str = cmd.program().to_string();
        let args_clone = cmd.get_args().to_vec();
        let descriptor = ProcessCommandDescriptor {
            program: command_str.clone(),
            args: args_clone.clone(),
            cwd: cmd.get_current_dir().cloned(),
        };

        // Emit operation started event
        emit_process_started(ctx, &descriptor).await;

        // Use the proven tokio Command execution pattern from the codebase
        let result: Result<CommandOutput, PlatformError> = async {
            let mut command = Command::new(cmd.program());
            command.args(cmd.get_args());

            if let Some(dir) = cmd.get_current_dir() {
                command.current_dir(dir);
            }

            // Set environment variables
            for (key, value) in cmd.get_env_vars() {
                command.env(key, value);
            }

            let output =
                command
                    .output()
                    .await
                    .map_err(|e| PlatformError::ProcessExecutionFailed {
                        command: cmd.program().to_string(),
                        message: e.to_string(),
                    })?;

            Ok(CommandOutput {
                status: output.status,
                stdout: output.stdout,
                stderr: output.stderr,
            })
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(output) => {
                emit_process_completed(ctx, &descriptor, output, duration).await;
            }
            Err(e) => {
                emit_process_failed(ctx, &descriptor, e, duration).await;
            }
        }

        result.map_err(Error::from)
    }

    fn create_command(&self, program: &str) -> PlatformCommand {
        PlatformCommand::new(program)
    }

    async fn which(&self, program: &str) -> Result<PathBuf, Error> {
        // Use the platform manager's tool registry for which command
        let platform_manager = crate::core::PlatformManager::instance();
        let which_path = platform_manager
            .get_tool("which")
            .await
            .map_err(Error::from)?;

        let output = Command::new(&which_path)
            .arg(program)
            .output()
            .await
            .map_err(|e| {
                Error::from(PlatformError::ProcessExecutionFailed {
                    command: "which".to_string(),
                    message: e.to_string(),
                })
            })?;

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
