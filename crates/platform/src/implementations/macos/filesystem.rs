//! macOS filesystem operations implementation
//!
//! This module wraps the existing proven filesystem operations from the root crate
//! with the platform abstraction layer, adding event emission and proper error handling.

use async_trait::async_trait;
use sps2_errors::PlatformError;
use sps2_events::{events::PlatformEvent, AppEvent};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Instant;
use tokio::fs;

use crate::core::PlatformContext;
use crate::filesystem::FilesystemOperations;

/// macOS implementation of filesystem operations
pub struct MacOSFilesystemOperations;

impl MacOSFilesystemOperations {
    pub fn new() -> Self {
        Self
    }

    /// Calculate the size of a file or directory recursively
    async fn calculate_size(&self, path: &Path) -> Result<u64, std::io::Error> {
        let metadata = tokio::fs::metadata(path).await?;

        if metadata.is_file() {
            Ok(metadata.len())
        } else if metadata.is_dir() {
            let mut total = 0u64;
            let mut entries = tokio::fs::read_dir(path).await?;

            while let Some(entry) = entries.next_entry().await? {
                let entry_path = entry.path();
                total += Box::pin(self.calculate_size(&entry_path)).await?;
            }

            Ok(total)
        } else {
            Ok(0) // Symlinks, devices, etc.
        }
    }
}

impl Default for MacOSFilesystemOperations {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FilesystemOperations for MacOSFilesystemOperations {
    async fn clone_file(
        &self,
        ctx: &PlatformContext,
        src: &Path,
        dst: &Path,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        let src_path = src.to_string_lossy().to_string();
        let dst_path = dst.to_string_lossy().to_string();

        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(
            PlatformEvent::FilesystemOperationStarted {
                operation: "clone_file".to_string(),
                source_path: Some(src_path.clone()),
                target_path: dst_path.clone(),
                context: std::collections::HashMap::new(),
            },
        ))
        .await;

        // Use the proven APFS clonefile implementation from root crate
        let result = async {
            // APFS clonefile constants
            const CLONE_NOFOLLOW: u32 = 0x0001;
            const CLONE_NOOWNERCOPY: u32 = 0x0002;

            let src_cstring = CString::new(src.as_os_str().as_bytes()).map_err(|_| {
                PlatformError::FilesystemOperationFailed {
                    operation: "clone_file".to_string(),
                    message: format!("Invalid source path: {}", src.display()),
                }
            })?;

            let dst_cstring = CString::new(dst.as_os_str().as_bytes()).map_err(|_| {
                PlatformError::FilesystemOperationFailed {
                    operation: "clone_file".to_string(),
                    message: format!("Invalid destination path: {}", dst.display()),
                }
            })?;

            tokio::task::spawn_blocking(move || {
                // SAFETY: clonefile is available on macOS and we're passing valid C strings
                unsafe {
                    let result = libc::clonefile(
                        src_cstring.as_ptr(),
                        dst_cstring.as_ptr(),
                        CLONE_NOFOLLOW | CLONE_NOOWNERCOPY,
                    );

                    if result != 0 {
                        let errno = *libc::__error();
                        return Err(PlatformError::FilesystemOperationFailed {
                            operation: "clone_file".to_string(),
                            message: format!(
                                "clonefile failed with code {result}, errno: {errno} ({})",
                                std::io::Error::from_raw_os_error(errno)
                            ),
                        });
                    }
                }
                Ok(())
            })
            .await
            .map_err(|e| PlatformError::FilesystemOperationFailed {
                operation: "clone_file".to_string(),
                message: format!("clone task failed: {e}"),
            })?
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationCompleted {
                        operation: "clone_file".to_string(),
                        paths_affected: vec![src_path, dst_path],
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationFailed {
                        operation: "clone_file".to_string(),
                        paths_involved: vec![src_path, dst_path],
                        error_message: e.to_string(),
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
        }

        result
    }

    async fn clone_directory(
        &self,
        ctx: &PlatformContext,
        src: &Path,
        dst: &Path,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        let src_path = src.to_string_lossy().to_string();
        let dst_path = dst.to_string_lossy().to_string();

        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(
            PlatformEvent::FilesystemOperationStarted {
                operation: "clone_directory".to_string(),
                source_path: Some(src_path.clone()),
                target_path: dst_path.clone(),
                context: std::collections::HashMap::new(),
            },
        ))
        .await;

        // Use the same clonefile implementation as clone_file since APFS clonefile handles directories
        let result = async {
            // APFS clonefile constants
            const CLONE_NOFOLLOW: u32 = 0x0001;
            const CLONE_NOOWNERCOPY: u32 = 0x0002;

            let src_cstring = CString::new(src.as_os_str().as_bytes()).map_err(|_| {
                PlatformError::FilesystemOperationFailed {
                    operation: "clone_directory".to_string(),
                    message: format!("Invalid source path: {}", src.display()),
                }
            })?;

            let dst_cstring = CString::new(dst.as_os_str().as_bytes()).map_err(|_| {
                PlatformError::FilesystemOperationFailed {
                    operation: "clone_directory".to_string(),
                    message: format!("Invalid destination path: {}", dst.display()),
                }
            })?;

            tokio::task::spawn_blocking(move || {
                // SAFETY: clonefile is available on macOS and we're passing valid C strings
                unsafe {
                    let result = libc::clonefile(
                        src_cstring.as_ptr(),
                        dst_cstring.as_ptr(),
                        CLONE_NOFOLLOW | CLONE_NOOWNERCOPY,
                    );

                    if result != 0 {
                        let errno = *libc::__error();
                        return Err(PlatformError::FilesystemOperationFailed {
                            operation: "clone_directory".to_string(),
                            message: format!(
                                "clonefile failed with code {result}, errno: {errno} ({})",
                                std::io::Error::from_raw_os_error(errno)
                            ),
                        });
                    }
                }
                Ok(())
            })
            .await
            .map_err(|e| PlatformError::FilesystemOperationFailed {
                operation: "clone_directory".to_string(),
                message: format!("clone task failed: {e}"),
            })?
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationCompleted {
                        operation: "clone_directory".to_string(),
                        paths_affected: vec![src_path, dst_path],
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationFailed {
                        operation: "clone_directory".to_string(),
                        paths_involved: vec![src_path, dst_path],
                        error_message: e.to_string(),
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
        }

        result
    }

    async fn atomic_rename(
        &self,
        ctx: &PlatformContext,
        src: &Path,
        dst: &Path,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        let src_path = src.to_string_lossy().to_string();
        let dst_path = dst.to_string_lossy().to_string();

        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(
            PlatformEvent::FilesystemOperationStarted {
                operation: "atomic_rename".to_string(),
                source_path: Some(src_path.clone()),
                target_path: dst_path.clone(),
                context: std::collections::HashMap::new(),
            },
        ))
        .await;

        // Use the proven atomic rename implementation from root crate
        let result = async {
            #[cfg(target_os = "macos")]
            {
                // Use async filesystem operations for proper directory handling
                if dst.exists() {
                    if dst.is_dir() {
                        // For directories, we need to remove the destination first
                        // Create a temporary backup location
                        let temp_dst = dst.with_extension("old");

                        // Move destination to temp location
                        fs::rename(dst, &temp_dst).await.map_err(|e| {
                            PlatformError::FilesystemOperationFailed {
                                operation: "atomic_rename".to_string(),
                                message: format!("failed to backup destination: {e}"),
                            }
                        })?;

                        // Move source to destination
                        match fs::rename(src, dst).await {
                            Ok(()) => {
                                // Success! Remove the old destination
                                let _ = fs::remove_dir_all(&temp_dst).await;
                                Ok(())
                            }
                            Err(e) => {
                                // Failed! Restore the original destination
                                let _ = fs::rename(&temp_dst, dst).await;
                                Err(PlatformError::FilesystemOperationFailed {
                                    operation: "atomic_rename".to_string(),
                                    message: format!("rename failed: {e}"),
                                })
                            }
                        }
                    } else {
                        // For files, regular rename should work
                        fs::rename(src, dst).await.map_err(|e| {
                            PlatformError::FilesystemOperationFailed {
                                operation: "atomic_rename".to_string(),
                                message: format!("rename failed: {e}"),
                            }
                        })
                    }
                } else {
                    // Destination doesn't exist, regular rename
                    fs::rename(src, dst).await.map_err(|e| {
                        PlatformError::FilesystemOperationFailed {
                            operation: "atomic_rename".to_string(),
                            message: format!("rename failed: {e}"),
                        }
                    })
                }
            }

            #[cfg(not(target_os = "macos"))]
            {
                // Fallback to regular rename (not truly atomic swap)
                fs::rename(src, dst)
                    .await
                    .map_err(|e| PlatformError::FilesystemOperationFailed {
                        operation: "atomic_rename".to_string(),
                        message: e.to_string(),
                    })
            }
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationCompleted {
                        operation: "atomic_rename".to_string(),
                        paths_affected: vec![src_path, dst_path],
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationFailed {
                        operation: "atomic_rename".to_string(),
                        paths_involved: vec![src_path, dst_path],
                        error_message: e.to_string(),
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
        }

        result
    }

    async fn atomic_swap(
        &self,
        ctx: &PlatformContext,
        path_a: &Path,
        path_b: &Path,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        let path_a_str = path_a.to_string_lossy().to_string();
        let path_b_str = path_b.to_string_lossy().to_string();

        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(
            PlatformEvent::FilesystemOperationStarted {
                operation: "atomic_swap".to_string(),
                source_path: Some(path_a_str.clone()),
                target_path: path_b_str.clone(),
                context: std::collections::HashMap::new(),
            },
        ))
        .await;

        // Use the proven atomic swap implementation from root crate
        let result = async {
            #[cfg(target_os = "macos")]
            {
                use libc::{c_uint, renamex_np, RENAME_SWAP};

                // Verify both paths exist before attempting swap
                if !path_a.exists() {
                    return Err(PlatformError::FilesystemOperationFailed {
                        operation: "atomic_swap".to_string(),
                        message: format!("Path does not exist: {}", path_a.display()),
                    });
                }
                if !path_b.exists() {
                    return Err(PlatformError::FilesystemOperationFailed {
                        operation: "atomic_swap".to_string(),
                        message: format!("Path does not exist: {}", path_b.display()),
                    });
                }

                let path1_cstring = CString::new(path_a.as_os_str().as_bytes()).map_err(|_| {
                    PlatformError::FilesystemOperationFailed {
                        operation: "atomic_swap".to_string(),
                        message: format!("Invalid path: {}", path_a.display()),
                    }
                })?;

                let path2_cstring = CString::new(path_b.as_os_str().as_bytes()).map_err(|_| {
                    PlatformError::FilesystemOperationFailed {
                        operation: "atomic_swap".to_string(),
                        message: format!("Invalid path: {}", path_b.display()),
                    }
                })?;

                tokio::task::spawn_blocking(move || {
                    #[allow(unsafe_code)]
                    // SAFETY: renamex_np is available on macOS and we're passing valid C strings
                    unsafe {
                        if renamex_np(
                            path1_cstring.as_ptr(),
                            path2_cstring.as_ptr(),
                            RENAME_SWAP as c_uint,
                        ) != 0
                        {
                            let err = std::io::Error::last_os_error();
                            return Err(PlatformError::FilesystemOperationFailed {
                                operation: "atomic_swap".to_string(),
                                message: format!("atomic swap failed: {err}"),
                            });
                        }
                    }
                    Ok(())
                })
                .await
                .map_err(|e| PlatformError::FilesystemOperationFailed {
                    operation: "atomic_swap".to_string(),
                    message: format!("swap task failed: {e}"),
                })?
            }

            #[cfg(not(target_os = "macos"))]
            {
                // No true atomic swap available on non-macOS platforms
                // This is a potentially unsafe fallback using temporary file
                let temp_path = path_a.with_extension("tmp_swap");

                fs::rename(path_a, &temp_path).await.map_err(|e| {
                    PlatformError::FilesystemOperationFailed {
                        operation: "atomic_swap".to_string(),
                        message: format!("temp rename failed: {e}"),
                    }
                })?;

                fs::rename(path_b, path_a).await.map_err(|e| {
                    PlatformError::FilesystemOperationFailed {
                        operation: "atomic_swap".to_string(),
                        message: format!("second rename failed: {e}"),
                    }
                })?;

                fs::rename(&temp_path, path_b).await.map_err(|e| {
                    PlatformError::FilesystemOperationFailed {
                        operation: "atomic_swap".to_string(),
                        message: format!("final rename failed: {e}"),
                    }
                })?;

                Ok(())
            }
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationCompleted {
                        operation: "atomic_swap".to_string(),
                        paths_affected: vec![path_a_str, path_b_str],
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationFailed {
                        operation: "atomic_swap".to_string(),
                        paths_involved: vec![path_a_str, path_b_str],
                        error_message: e.to_string(),
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
        }

        result
    }

    async fn hard_link(
        &self,
        ctx: &PlatformContext,
        src: &Path,
        dst: &Path,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        let src_path = src.to_string_lossy().to_string();
        let dst_path = dst.to_string_lossy().to_string();

        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(
            PlatformEvent::FilesystemOperationStarted {
                operation: "hard_link".to_string(),
                source_path: Some(src_path.clone()),
                target_path: dst_path.clone(),
                context: std::collections::HashMap::new(),
            },
        ))
        .await;

        // Use the proven hard link implementation from root crate
        let result = async {
            #[cfg(target_os = "macos")]
            {
                let src_cstring = CString::new(src.as_os_str().as_bytes()).map_err(|_| {
                    PlatformError::FilesystemOperationFailed {
                        operation: "hard_link".to_string(),
                        message: format!("Invalid source path: {}", src.display()),
                    }
                })?;

                let dst_cstring = CString::new(dst.as_os_str().as_bytes()).map_err(|_| {
                    PlatformError::FilesystemOperationFailed {
                        operation: "hard_link".to_string(),
                        message: format!("Invalid destination path: {}", dst.display()),
                    }
                })?;

                tokio::task::spawn_blocking(move || {
                    let result = unsafe { libc::link(src_cstring.as_ptr(), dst_cstring.as_ptr()) };
                    if result != 0 {
                        let errno = unsafe { *libc::__error() };
                        return Err(PlatformError::FilesystemOperationFailed {
                            operation: "hard_link".to_string(),
                            message: format!(
                                "hard link failed with code {result}, errno: {errno} ({})",
                                std::io::Error::from_raw_os_error(errno)
                            ),
                        });
                    }
                    Ok(())
                })
                .await
                .map_err(|e| PlatformError::FilesystemOperationFailed {
                    operation: "hard_link".to_string(),
                    message: format!("hard link task failed: {e}"),
                })?
            }

            #[cfg(not(target_os = "macos"))]
            {
                fs::hard_link(src, dst).await.map_err(|e| {
                    PlatformError::FilesystemOperationFailed {
                        operation: "hard_link".to_string(),
                        message: format!("hard link failed: {e}"),
                    }
                })
            }
        }
        .await;

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationCompleted {
                        operation: "hard_link".to_string(),
                        paths_affected: vec![src_path, dst_path],
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationFailed {
                        operation: "hard_link".to_string(),
                        paths_involved: vec![src_path, dst_path],
                        error_message: e.to_string(),
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
        }

        result
    }

    async fn create_dir_all(
        &self,
        ctx: &PlatformContext,
        path: &Path,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        let path_str = path.to_string_lossy().to_string();

        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(
            PlatformEvent::FilesystemOperationStarted {
                operation: "create_dir_all".to_string(),
                source_path: None,
                target_path: path_str.clone(),
                context: std::collections::HashMap::new(),
            },
        ))
        .await;

        // Use standard tokio::fs implementation
        let result =
            fs::create_dir_all(path)
                .await
                .map_err(|e| PlatformError::FilesystemOperationFailed {
                    operation: "create_dir_all".to_string(),
                    message: format!("create directory failed: {e}"),
                });

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationCompleted {
                        operation: "create_dir_all".to_string(),
                        paths_affected: vec![path_str],
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationFailed {
                        operation: "create_dir_all".to_string(),
                        paths_involved: vec![path_str],
                        error_message: e.to_string(),
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
        }

        result
    }

    async fn remove_dir_all(
        &self,
        ctx: &PlatformContext,
        path: &Path,
    ) -> Result<(), PlatformError> {
        let start = Instant::now();
        let path_str = path.to_string_lossy().to_string();

        // Emit operation started event
        ctx.emit_event(AppEvent::Platform(
            PlatformEvent::FilesystemOperationStarted {
                operation: "remove_dir_all".to_string(),
                source_path: None,
                target_path: path_str.clone(),
                context: std::collections::HashMap::new(),
            },
        ))
        .await;

        // Use standard tokio::fs implementation
        let result =
            fs::remove_dir_all(path)
                .await
                .map_err(|e| PlatformError::FilesystemOperationFailed {
                    operation: "remove_dir_all".to_string(),
                    message: format!("remove directory failed: {e}"),
                });

        let duration = start.elapsed();

        // Emit completion event
        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationCompleted {
                        operation: "remove_dir_all".to_string(),
                        paths_affected: vec![path_str],
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationFailed {
                        operation: "remove_dir_all".to_string(),
                        paths_involved: vec![path_str],
                        error_message: e.to_string(),
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
        }

        result
    }

    /// Check if a path exists
    async fn exists(&self, _ctx: &PlatformContext, path: &Path) -> bool {
        tokio::fs::metadata(path).await.is_ok()
    }

    /// Remove a single file
    async fn remove_file(&self, ctx: &PlatformContext, path: &Path) -> Result<(), PlatformError> {
        let start = Instant::now();
        let path_str = path.display().to_string();

        ctx.emit_event(AppEvent::Platform(
            PlatformEvent::FilesystemOperationStarted {
                operation: "remove_file".to_string(),
                source_path: None,
                target_path: path_str.clone(),
                context: HashMap::new(),
            },
        ))
        .await;

        let result = tokio::fs::remove_file(path).await.map_err(|e| {
            PlatformError::FilesystemOperationFailed {
                operation: "remove_file".to_string(),
                message: e.to_string(),
            }
        });

        let duration = start.elapsed();

        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationCompleted {
                        operation: "remove_file".to_string(),
                        paths_affected: vec![path_str],
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationFailed {
                        operation: "remove_file".to_string(),
                        paths_involved: vec![path_str],
                        error_message: e.to_string(),
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
        }

        result
    }

    /// Get the size of a file or directory
    async fn size(&self, ctx: &PlatformContext, path: &Path) -> Result<u64, PlatformError> {
        let start = Instant::now();
        let path_str = path.display().to_string();

        ctx.emit_event(AppEvent::Platform(
            PlatformEvent::FilesystemOperationStarted {
                operation: "size".to_string(),
                source_path: None,
                target_path: path_str.clone(),
                context: HashMap::new(),
            },
        ))
        .await;

        let result =
            self.calculate_size(path)
                .await
                .map_err(|e| PlatformError::FilesystemOperationFailed {
                    operation: "size".to_string(),
                    message: e.to_string(),
                });

        let duration = start.elapsed();

        match &result {
            Ok(_) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationCompleted {
                        operation: "size".to_string(),
                        paths_affected: vec![path_str],
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
            Err(e) => {
                ctx.emit_event(AppEvent::Platform(
                    PlatformEvent::FilesystemOperationFailed {
                        operation: "size".to_string(),
                        paths_involved: vec![path_str],
                        error_message: e.to_string(),
                        duration_ms: duration.as_millis() as u64,
                    },
                ))
                .await;
            }
        }

        result
    }
}
