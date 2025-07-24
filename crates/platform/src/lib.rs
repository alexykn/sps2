//! Platform abstraction layer for macOS ARM64 package manager operations.
//!
//! This crate provides a unified interface for platform-specific operations including:
//! - Binary operations (install_name_tool, otool, codesign)
//! - Filesystem operations (APFS clonefile, atomic operations)
//! - Process execution with proper event emission and error handling
//!
//! The platform abstraction integrates seamlessly with the existing event system
//! and error handling patterns in the sps2 codebase.

pub mod binary;
pub mod core;
pub mod filesystem;
pub mod implementations;
pub mod process;

pub use core::{Platform, PlatformContext};
pub use implementations::macos::MacOSPlatform;

/// Re-export commonly used types
pub use binary::BinaryOperations;
pub use filesystem::FilesystemOperations;
pub use process::ProcessOperations;