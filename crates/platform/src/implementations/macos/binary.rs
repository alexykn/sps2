//! macOS binary operations implementation
//! 
//! This module wraps the existing proven binary operations from RPathPatcher and CodeSigner
//! with the platform abstraction layer, adding event emission and proper error handling.

use async_trait::async_trait;
use std::path::Path;
use std::time::Instant;
use sps2_errors::PlatformError;
use sps2_events::{AppEvent, events::PlatformEvent};
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

#[async_trait]
impl BinaryOperations for MacOSBinaryOperations {
    async fn get_install_name(&self, ctx: &PlatformContext, binary: &Path) -> Result<Option<String>, PlatformError> {
        let start = Instant::now();
        let binary_path = binary.to_string_lossy().to_string();
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationStarted {
            operation: "get_install_name".to_string(),
            binary_path: binary_path.clone(),
            context: std::collections::HashMap::new(),
        })).await;

        // Use the proven otool -D implementation from RPathPatcher
        let result: Result<Option<String>, PlatformError> = async {
            let out = Command::new("otool")
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
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationCompleted {
                    operation: "get_install_name".to_string(),
                    binary_path,
                    changes_made: vec![],
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationFailed {
                    operation: "get_install_name".to_string(),
                    binary_path,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result
    }
    
    async fn set_install_name(&self, ctx: &PlatformContext, binary: &Path, name: &str) -> Result<(), PlatformError> {
        let start = Instant::now();
        let binary_path = binary.to_string_lossy().to_string();
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationStarted {
            operation: "set_install_name".to_string(),
            binary_path: binary_path.clone(),
            context: [("new_name".to_string(), name.to_string())].into(),
        })).await;

        // Use the proven install_name_tool implementation from RPathPatcher
        let result = async {
            let output = Command::new("install_name_tool")
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
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationCompleted {
                    operation: "set_install_name".to_string(),
                    binary_path,
                    changes_made: vec![format!("Set install name to: {}", name)],
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationFailed {
                    operation: "set_install_name".to_string(),
                    binary_path,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result
    }
    
    async fn get_dependencies(&self, ctx: &PlatformContext, binary: &Path) -> Result<Vec<String>, PlatformError> {
        let start = Instant::now();
        let binary_path = binary.to_string_lossy().to_string();
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationStarted {
            operation: "get_dependencies".to_string(),
            binary_path: binary_path.clone(),
            context: std::collections::HashMap::new(),
        })).await;

        // Use the proven otool -L implementation from RPathPatcher
        let result = async {
            let output = Command::new("otool")
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
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationCompleted {
                    operation: "get_dependencies".to_string(),
                    binary_path,
                    changes_made: vec![],
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationFailed {
                    operation: "get_dependencies".to_string(),
                    binary_path,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result
    }
    
    async fn change_dependency(&self, ctx: &PlatformContext, binary: &Path, old: &str, new: &str) -> Result<(), PlatformError> {
        let start = Instant::now();
        let binary_path = binary.to_string_lossy().to_string();
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationStarted {
            operation: "change_dependency".to_string(),
            binary_path: binary_path.clone(),
            context: [
                ("old_dependency".to_string(), old.to_string()),
                ("new_dependency".to_string(), new.to_string()),
            ].into(),
        })).await;

        // Use the proven install_name_tool -change implementation from RPathPatcher
        let result = async {
            let change_output = Command::new("install_name_tool")
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
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationCompleted {
                    operation: "change_dependency".to_string(),
                    binary_path,
                    changes_made: vec![format!("Changed dependency: {} -> {}", old, new)],
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationFailed {
                    operation: "change_dependency".to_string(),
                    binary_path,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result
    }
    
    async fn add_rpath(&self, ctx: &PlatformContext, binary: &Path, rpath: &str) -> Result<(), PlatformError> {
        let start = Instant::now();
        let binary_path = binary.to_string_lossy().to_string();
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationStarted {
            operation: "add_rpath".to_string(),
            binary_path: binary_path.clone(),
            context: [("rpath".to_string(), rpath.to_string())].into(),
        })).await;

        // Use the proven install_name_tool -add_rpath implementation from RPathPatcher
        let result = async {
            let output = Command::new("install_name_tool")
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
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationCompleted {
                    operation: "add_rpath".to_string(),
                    binary_path,
                    changes_made: vec![format!("Added rpath: {}", rpath)],
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationFailed {
                    operation: "add_rpath".to_string(),
                    binary_path,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result
    }
    
    async fn delete_rpath(&self, ctx: &PlatformContext, binary: &Path, rpath: &str) -> Result<(), PlatformError> {
        let start = Instant::now();
        let binary_path = binary.to_string_lossy().to_string();
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationStarted {
            operation: "delete_rpath".to_string(),
            binary_path: binary_path.clone(),
            context: [("rpath".to_string(), rpath.to_string())].into(),
        })).await;

        // Use the proven install_name_tool -delete_rpath implementation from RPathPatcher
        let result = async {
            let output = Command::new("install_name_tool")
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
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationCompleted {
                    operation: "delete_rpath".to_string(),
                    binary_path,
                    changes_made: vec![format!("Deleted rpath: {}", rpath)],
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationFailed {
                    operation: "delete_rpath".to_string(),
                    binary_path,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result
    }
    
    async fn get_rpath_entries(&self, ctx: &PlatformContext, binary: &Path) -> Result<Vec<String>, PlatformError> {
        let start = Instant::now();
        let binary_path = binary.to_string_lossy().to_string();
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationStarted {
            operation: "get_rpath_entries".to_string(),
            binary_path: binary_path.clone(),
            context: std::collections::HashMap::new(),
        })).await;

        // Use the proven otool -l implementation from RPathPatcher
        let result = async {
            let out = Command::new("otool")
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
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationCompleted {
                    operation: "get_rpath_entries".to_string(),
                    binary_path,
                    changes_made: vec![],
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationFailed {
                    operation: "get_rpath_entries".to_string(),
                    binary_path,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result
    }
    
    async fn verify_signature(&self, ctx: &PlatformContext, binary: &Path) -> Result<bool, PlatformError> {
        let start = Instant::now();
        let binary_path = binary.to_string_lossy().to_string();
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationStarted {
            operation: "verify_signature".to_string(),
            binary_path: binary_path.clone(),
            context: std::collections::HashMap::new(),
        })).await;

        // Use the proven codesign verification implementation from CodeSigner
        let result: Result<bool, PlatformError> = async {
            let check = Command::new("codesign")
                .args(["-vvv", &binary.to_string_lossy()])
                .output()
                .await
                .map_err(|e| PlatformError::ProcessExecutionFailed {
                    command: "codesign -vvv".to_string(),
                    message: e.to_string(),
                })?;

            Ok(check.status.success())
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(valid) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationCompleted {
                    operation: "verify_signature".to_string(),
                    binary_path,
                    changes_made: vec![format!("Signature valid: {}", valid)],
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationFailed {
                    operation: "verify_signature".to_string(),
                    binary_path,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result
    }
    
    async fn sign_binary(&self, ctx: &PlatformContext, binary: &Path, identity: Option<&str>) -> Result<(), PlatformError> {
        let start = Instant::now();
        let binary_path = binary.to_string_lossy().to_string();
        let identity_str = identity.unwrap_or("-");
        
        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationStarted {
            operation: "sign_binary".to_string(),
            binary_path: binary_path.clone(),
            context: [("identity".to_string(), identity_str.to_string())].into(),
        })).await;

        // Use the proven codesign implementation from CodeSigner
        let result = async {
            let output = Command::new("codesign")
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
        }.await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationCompleted {
                    operation: "sign_binary".to_string(),
                    binary_path,
                    changes_made: vec![format!("Signed with identity: {}", identity_str)],
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(PlatformEvent::BinaryOperationFailed {
                    operation: "sign_binary".to_string(),
                    binary_path,
                    error_message: e.to_string(),
                    duration_ms: duration.as_millis() as u64,
                })).await;
            }
        }

        result
    }
}