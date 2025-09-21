//! macOS binary operations implementation
//!
//! This module wraps the existing proven binary operations from RPathPatcher and CodeSigner
//! with the platform abstraction layer, adding event emission and proper error handling.

use async_trait::async_trait;
use sps2_errors::PlatformError;
use sps2_events::{
    events::{
        FailureContext, PlatformEvent, PlatformOperationContext, PlatformOperationKind,
        PlatformOperationMetrics,
    },
    AppEvent,
};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::process::Command;

use crate::binary::BinaryOperations;
use crate::core::PlatformContext;

/// macOS implementation of binary operations
pub struct MacOSBinaryOperations;

impl MacOSBinaryOperations {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MacOSBinaryOperations {
    fn default() -> Self {
        Self::new()
    }
}

fn binary_context(operation: &str, target: &Path) -> PlatformOperationContext {
    PlatformOperationContext {
        kind: PlatformOperationKind::Binary,
        operation: operation.to_string(),
        target: Some(target.to_path_buf()),
        source: None,
        command: None,
    }
}

fn binary_metrics(duration: Duration, changes: Option<Vec<String>>) -> PlatformOperationMetrics {
    PlatformOperationMetrics {
        duration_ms: Some(duration_to_millis(duration)),
        exit_code: None,
        stdout_bytes: None,
        stderr_bytes: None,
        changes,
    }
}

fn duration_to_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

async fn emit_binary_started(ctx: &PlatformContext, operation: &str, target: &Path) {
    ctx.emit_event(AppEvent::Platform(PlatformEvent::OperationStarted {
        context: binary_context(operation, target),
    }))
    .await;
}

async fn emit_binary_completed(
    ctx: &PlatformContext,
    operation: &str,
    target: &Path,
    changes: Option<Vec<String>>,
    duration: Duration,
) {
    ctx.emit_event(AppEvent::Platform(PlatformEvent::OperationCompleted {
        context: binary_context(operation, target),
        metrics: Some(binary_metrics(duration, changes)),
    }))
    .await;
}

async fn emit_binary_failed(
    ctx: &PlatformContext,
    operation: &str,
    target: &Path,
    error: &PlatformError,
    duration: Duration,
) {
    ctx.emit_event(AppEvent::Platform(PlatformEvent::OperationFailed {
        context: binary_context(operation, target),
        failure: FailureContext::from_error(error),
        metrics: Some(binary_metrics(duration, None)),
    }))
    .await;
}

#[async_trait]
impl BinaryOperations for MacOSBinaryOperations {
    async fn get_install_name(
        &self,
        ctx: &PlatformContext,
        binary: &Path,
    ) -> Result<Option<String>, PlatformError> {
        let start = Instant::now();
        // Emit operation started event
        emit_binary_started(ctx, "get_install_name", binary).await;

        // Use tool registry to get otool path
        let result: Result<Option<String>, PlatformError> = async {
            let otool_path = ctx.platform_manager().get_tool("otool").await?;

            let out = Command::new(&otool_path)
                .args(["-D", &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "otool -D".to_string(),
                    message: e.to_string(),
                })?;

            if !out.status.success() {
                return Ok(None);
            }

            let text = String::from_utf8_lossy(&out.stdout);
            // otool -D outputs:
            // /path/to/file:
            // install_name
            let lines: Vec<&str> = text.lines().collect();
            if lines.len() >= 2 {
                Ok(Some(lines[1].trim().to_string()))
            } else {
                Ok(None)
            }
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                emit_binary_completed(ctx, "get_install_name", binary, None, duration).await;
            }
            Err(e) => {
                emit_binary_failed(ctx, "get_install_name", binary, e, duration).await;
            }
        }

        result
    }

    async fn set_install_name(
        &self,
        ctx: &PlatformContext,
        binary: &Path,
        name: &str,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        // Emit operation started event
        emit_binary_started(ctx, "set_install_name", binary).await;

        // Use tool registry to get install_name_tool path
        let result = async {
            let install_name_tool_path =
                ctx.platform_manager().get_tool("install_name_tool").await?;

            let output = Command::new(&install_name_tool_path)
                .args(["-id", name, &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "install_name_tool -id".to_string(),
                    message: e.to_string(),
                })?;

            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);

                // Check for headerpad error (from existing RPathPatcher logic)
                if stderr.contains("larger updated load commands do not fit") {
                    Err(PlatformError::BinaryOperationFailed {
                        operation: "set_install_name".to_string(),
                        binary_path: binary.display().to_string(),
                        message: format!("HEADERPAD_ERROR: {}", binary.display()),
                    })
                } else {
                    Err(PlatformError::BinaryOperationFailed {
                        operation: "set_install_name".to_string(),
                        binary_path: binary.display().to_string(),
                        message: stderr.trim().to_string(),
                    })
                }
            }
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                emit_binary_completed(
                    ctx,
                    "set_install_name",
                    binary,
                    Some(vec![format!("set_install_name -> {name}")]),
                    duration,
                )
                .await;
            }
            Err(e) => {
                emit_binary_failed(ctx, "set_install_name", binary, e, duration).await;
            }
        }

        result
    }

    async fn get_dependencies(
        &self,
        ctx: &PlatformContext,
        binary: &Path,
    ) -> Result<Vec<String>, PlatformError> {
        let start = Instant::now();
        emit_binary_started(ctx, "get_dependencies", binary).await;

        // Use tool registry to get otool path
        let result = async {
            let otool_path = ctx.platform_manager().get_tool("otool").await?;

            let output = Command::new(&otool_path)
                .args(["-L", &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "otool -L".to_string(),
                    message: e.to_string(),
                })?;

            if !output.status.success() {
                return Err(PlatformError::BinaryOperationFailed {
                    operation: "get_dependencies".to_string(),
                    binary_path: binary.display().to_string(),
                    message: "otool -L failed".to_string(),
                });
            }

            let deps = String::from_utf8_lossy(&output.stdout);
            let mut dependencies = Vec::new();

            // Process each dependency line (skip the first line which is the file name)
            for line in deps.lines().skip(1) {
                let dep = line.trim();
                if let Some(dep_name) = dep.split_whitespace().next() {
                    dependencies.push(dep_name.to_string());
                }
            }

            Ok(dependencies)
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                emit_binary_completed(ctx, "get_dependencies", binary, None, duration).await;
            }
            Err(e) => {
                emit_binary_failed(ctx, "get_dependencies", binary, e, duration).await;
            }
        }

        result
    }

    async fn change_dependency(
        &self,
        ctx: &PlatformContext,
        binary: &Path,
        old: &str,
        new: &str,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        emit_binary_started(ctx, "change_dependency", binary).await;

        // Use the proven install_name_tool -change implementation from RPathPatcher
        let result = async {
            let install_name_tool_path =
                ctx.platform_manager().get_tool("install_name_tool").await?;
            let change_output = Command::new(&install_name_tool_path)
                .args(["-change", old, new, &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "install_name_tool -change".to_string(),
                    message: e.to_string(),
                })?;

            if change_output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&change_output.stderr);
                Err(PlatformError::BinaryOperationFailed {
                    operation: "change_dependency".to_string(),
                    binary_path: binary.display().to_string(),
                    message: stderr.trim().to_string(),
                })
            }
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                emit_binary_completed(
                    ctx,
                    "change_dependency",
                    binary,
                    Some(vec![format!("change_dependency {old} -> {new}")]),
                    duration,
                )
                .await;
            }
            Err(e) => {
                emit_binary_failed(ctx, "change_dependency", binary, e, duration).await;
            }
        }

        result
    }

    async fn add_rpath(
        &self,
        ctx: &PlatformContext,
        binary: &Path,
        rpath: &str,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        emit_binary_started(ctx, "add_rpath", binary).await;

        // Use the proven install_name_tool -add_rpath implementation from RPathPatcher
        let result = async {
            let install_name_tool_path =
                ctx.platform_manager().get_tool("install_name_tool").await?;
            let output = Command::new(&install_name_tool_path)
                .args(["-add_rpath", rpath, &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "install_name_tool -add_rpath".to_string(),
                    message: e.to_string(),
                })?;

            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(PlatformError::BinaryOperationFailed {
                    operation: "add_rpath".to_string(),
                    binary_path: binary.display().to_string(),
                    message: stderr.trim().to_string(),
                })
            }
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                emit_binary_completed(
                    ctx,
                    "add_rpath",
                    binary,
                    Some(vec![format!("add_rpath {rpath}")]),
                    duration,
                )
                .await;
            }
            Err(e) => {
                emit_binary_failed(ctx, "add_rpath", binary, e, duration).await;
            }
        }

        result
    }

    async fn delete_rpath(
        &self,
        ctx: &PlatformContext,
        binary: &Path,
        rpath: &str,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        emit_binary_started(ctx, "delete_rpath", binary).await;

        // Use the proven install_name_tool -delete_rpath implementation from RPathPatcher
        let result = async {
            let install_name_tool_path =
                ctx.platform_manager().get_tool("install_name_tool").await?;
            let output = Command::new(&install_name_tool_path)
                .args(["-delete_rpath", rpath, &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "install_name_tool -delete_rpath".to_string(),
                    message: e.to_string(),
                })?;

            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(PlatformError::BinaryOperationFailed {
                    operation: "delete_rpath".to_string(),
                    binary_path: binary.display().to_string(),
                    message: stderr.trim().to_string(),
                })
            }
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                emit_binary_completed(
                    ctx,
                    "delete_rpath",
                    binary,
                    Some(vec![format!("delete_rpath {rpath}")]),
                    duration,
                )
                .await;
            }
            Err(e) => {
                emit_binary_failed(ctx, "delete_rpath", binary, e, duration).await;
            }
        }

        result
    }

    async fn get_rpath_entries(
        &self,
        ctx: &PlatformContext,
        binary: &Path,
    ) -> Result<Vec<String>, PlatformError> {
        let start = Instant::now();
        emit_binary_started(ctx, "get_rpath_entries", binary).await;

        // Use the proven otool -l implementation from RPathPatcher
        let result = async {
            let otool_path = ctx.platform_manager().get_tool("otool").await?;
            let out = Command::new(&otool_path)
                .args(["-l", &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "otool -l".to_string(),
                    message: e.to_string(),
                })?;

            if !out.status.success() {
                return Err(PlatformError::BinaryOperationFailed {
                    operation: "get_rpath_entries".to_string(),
                    binary_path: binary.display().to_string(),
                    message: "otool -l failed".to_string(),
                });
            }

            let text = String::from_utf8_lossy(&out.stdout);
            let mut rpath_entries = Vec::new();

            // Parse the LC_RPATH entries (logic from RPathPatcher)
            let mut lines = text.lines();
            while let Some(l) = lines.next() {
                if l.contains("LC_RPATH") {
                    let _ = lines.next(); // skip cmdsize
                    if let Some(p) = lines.next() {
                        if let Some(idx) = p.find("path ") {
                            let rpath = &p[idx + 5..p.find(" (").unwrap_or(p.len())];
                            rpath_entries.push(rpath.to_string());
                        }
                    }
                }
            }

            Ok(rpath_entries)
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                emit_binary_completed(ctx, "get_rpath_entries", binary, None, duration).await;
            }
            Err(e) => {
                emit_binary_failed(ctx, "get_rpath_entries", binary, e, duration).await;
            }
        }

        result
    }

    async fn verify_signature(
        &self,
        ctx: &PlatformContext,
        binary: &Path,
    ) -> Result<bool, PlatformError> {
        let start = Instant::now();
        emit_binary_started(ctx, "verify_signature", binary).await;

        // Use the proven codesign verification implementation from CodeSigner
        let result: Result<bool, PlatformError> = async {
            let codesign_path = ctx.platform_manager().get_tool("codesign").await?;
            let check = Command::new(&codesign_path)
                .args(["-vvv", &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "codesign -vvv".to_string(),
                    message: e.to_string(),
                })?;

            Ok(check.status.success())
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(valid) => {
                emit_binary_completed(
                    ctx,
                    "verify_signature",
                    binary,
                    Some(vec![format!("signature_valid={valid}")]),
                    duration,
                )
                .await;
            }
            Err(e) => {
                emit_binary_failed(ctx, "verify_signature", binary, e, duration).await;
            }
        }

        result
    }

    async fn sign_binary(
        &self,
        ctx: &PlatformContext,
        binary: &Path,
        identity: Option<&str>,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        let identity_str = identity.unwrap_or("-");

        emit_binary_started(ctx, "sign_binary", binary).await;

        // Use the proven codesign implementation from CodeSigner
        let result = async {
            let codesign_path = ctx.platform_manager().get_tool("codesign").await?;
            let output = Command::new(&codesign_path)
                .args(["-f", "-s", identity_str, &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "codesign".to_string(),
                    message: e.to_string(),
                })?;

            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(PlatformError::SigningFailed {
                    binary_path: binary.display().to_string(),
                    message: stderr.trim().to_string(),
                })
            }
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                emit_binary_completed(
                    ctx,
                    "sign_binary",
                    binary,
                    Some(vec![format!("sign_binary {identity_str}")]),
                    duration,
                )
                .await;
            }
            Err(e) => {
                emit_binary_failed(ctx, "sign_binary", binary, e, duration).await;
            }
        }

        result
    }
}
