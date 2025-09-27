//! Filesystem operations for macOS platform (APFS clonefile, atomic operations)

use async_trait::async_trait;
use sps2_errors::PlatformError;
use std::path::Path;

use crate::core::PlatformContext;

/// Trait for filesystem operations specific to macOS
#[async_trait]
pub trait FilesystemOperations: Send + Sync {
    /// Clone a file using APFS clonefile for efficient copy-on-write
    async fn clone_file(
        &self,
        ctx: &PlatformContext,
        src: &Path,
        dst: &Path,
    ) -> Result<(), PlatformError>;

    /// Clone a directory using APFS clonefile for efficient copy-on-write
    async fn clone_directory(
        &self,
        ctx: &PlatformContext,
        src: &Path,
        dst: &Path,
    ) -> Result<(), PlatformError>;

    /// Atomically rename a file
    async fn atomic_rename(
        &self,
        ctx: &PlatformContext,
        src: &Path,
        dst: &Path,
    ) -> Result<(), PlatformError>;

    /// Atomically swap two files
    async fn atomic_swap(
        &self,
        ctx: &PlatformContext,
        path_a: &Path,
        path_b: &Path,
    ) -> Result<(), PlatformError>;

    /// Create a hard link between files
    async fn hard_link(
        &self,
        ctx: &PlatformContext,
        src: &Path,
        dst: &Path,
    ) -> Result<(), PlatformError>;

    /// Create directory and all parent directories
    async fn create_dir_all(&self, ctx: &PlatformContext, path: &Path)
        -> Result<(), PlatformError>;

    /// Remove directory and all contents
    async fn remove_dir_all(&self, ctx: &PlatformContext, path: &Path)
        -> Result<(), PlatformError>;

    /// Check if a path exists
    async fn exists(&self, ctx: &PlatformContext, path: &Path) -> bool;

    /// Remove a single file
    async fn remove_file(&self, ctx: &PlatformContext, path: &Path) -> Result<(), PlatformError>;

    /// Get the size of a file or directory
    async fn size(&self, ctx: &PlatformContext, path: &Path) -> Result<u64, PlatformError>;

    /// Check if a path points to a directory.
    async fn is_dir(&self, ctx: &PlatformContext, path: &Path) -> bool;
}
