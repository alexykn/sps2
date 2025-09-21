//! macOS filesystem operations implementation
//!
//! This module wraps the existing proven filesystem operations from the root crate
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

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};
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

fn filesystem_context(
    operation: &str,
    source: Option<&Path>,
    target: &Path,
) -> PlatformOperationContext {
    PlatformOperationContext {
        kind: PlatformOperationKind::Filesystem,
        operation: operation.to_string(),
        target: Some(target.to_path_buf()),
        source: source.map(Path::to_path_buf),
        command: None,
    }
}

fn filesystem_metrics(
    duration: Duration,
    changes: Option<Vec<String>>,
) -> PlatformOperationMetrics {
    PlatformOperationMetrics {
        duration_ms: Some(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
        exit_code: None,
        stdout_bytes: None,
        stderr_bytes: None,
        changes,
    }
}

async fn emit_fs_started(
    ctx: &PlatformContext,
    operation: &str,
    source: Option<&Path>,
    target: &Path,
) {
    ctx.emit_event(AppEvent::Platform(PlatformEvent::OperationStarted {
        context: filesystem_context(operation, source, target),
    }))
    .await;
}

async fn emit_fs_completed(
    ctx: &PlatformContext,
    operation: &str,
    source: Option<&Path>,
    target: &Path,
    changes: Option<Vec<String>>,
    duration: Duration,
) {
    ctx.emit_event(AppEvent::Platform(PlatformEvent::OperationCompleted {
        context: filesystem_context(operation, source, target),
        metrics: Some(filesystem_metrics(duration, changes)),
    }))
    .await;
}

async fn emit_fs_failed(
    ctx: &PlatformContext,
    operation: &str,
    source: Option<&Path>,
    target: &Path,
    error: &PlatformError,
    duration: Duration,
) {
    ctx.emit_event(AppEvent::Platform(PlatformEvent::OperationFailed {
        context: filesystem_context(operation, source, target),
        failure: FailureContext::from_error(error),
        metrics: Some(filesystem_metrics(duration, None)),
    }))
    .await;
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
        emit_fs_started(ctx, "clone_file", Some(src), dst).await;

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
                emit_fs_completed(
                    ctx,
                    "clone_file",
                    Some(src),
                    dst,
                    Some(vec![format!("{} -> {}", src.display(), dst.display())]),
                    duration,
                )
                .await;
            }
            Err(e) => {
                emit_fs_failed(ctx, "clone_file", Some(src), dst, e, duration).await;
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
        emit_fs_started(ctx, "clone_directory", Some(src), dst).await;

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
                emit_fs_completed(ctx, "clone_directory", Some(src), dst, None, duration).await;
            }
            Err(e) => {
                emit_fs_failed(ctx, "clone_directory", Some(src), dst, e, duration).await;
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
        emit_fs_started(ctx, "atomic_rename", Some(src), dst).await;

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
                emit_fs_completed(ctx, "atomic_rename", Some(src), dst, None, duration).await;
            }
            Err(e) => {
                emit_fs_failed(ctx, "atomic_rename", Some(src), dst, e, duration).await;
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
        emit_fs_started(ctx, "atomic_swap", Some(path_a), path_b).await;

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
                emit_fs_completed(ctx, "atomic_swap", Some(path_a), path_b, None, duration).await;
            }
            Err(e) => {
                emit_fs_failed(ctx, "atomic_swap", Some(path_a), path_b, e, duration).await;
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
        emit_fs_started(ctx, "hard_link", Some(src), dst).await;

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
                emit_fs_completed(ctx, "hard_link", Some(src), dst, None, duration).await;
            }
            Err(e) => {
                emit_fs_failed(ctx, "hard_link", Some(src), dst, e, duration).await;
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
        emit_fs_started(ctx, "create_dir_all", None, path).await;

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
                emit_fs_completed(ctx, "create_dir_all", None, path, None, duration).await;
            }
            Err(e) => {
                emit_fs_failed(ctx, "create_dir_all", None, path, e, duration).await;
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
        emit_fs_started(ctx, "remove_dir_all", None, path).await;

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
                emit_fs_completed(ctx, "remove_dir_all", None, path, None, duration).await;
            }
            Err(e) => {
                emit_fs_failed(ctx, "remove_dir_all", None, path, e, duration).await;
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
        emit_fs_started(ctx, "remove_file", None, path).await;

        let result = tokio::fs::remove_file(path).await.map_err(|e| {
            PlatformError::FilesystemOperationFailed {
                operation: "remove_file".to_string(),
                message: e.to_string(),
            }
        });

        let duration = start.elapsed();

        match &result {
            Ok(_) => {
                emit_fs_completed(ctx, "remove_file", None, path, None, duration).await;
            }
            Err(e) => {
                emit_fs_failed(ctx, "remove_file", None, path, e, duration).await;
            }
        }

        result
    }

    /// Get the size of a file or directory
    async fn size(&self, ctx: &PlatformContext, path: &Path) -> Result<u64, PlatformError> {
        let start = Instant::now();
        emit_fs_started(ctx, "size", None, path).await;

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
                emit_fs_completed(ctx, "size", None, path, None, duration).await;
            }
            Err(e) => {
                emit_fs_failed(ctx, "size", None, path, e, duration).await;
            }
        }

        result
    }
}
