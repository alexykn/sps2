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
}
