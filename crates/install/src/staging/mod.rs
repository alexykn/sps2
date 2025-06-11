//! Staging directory management for secure package extraction
//!
//! This module provides secure staging directory creation, validation, and cleanup
//! for package installation. It ensures that packages are extracted to temporary
//! directories, validated, and then atomically moved to their final location.

pub mod directory;
pub mod guard;
pub mod manager;
pub mod utils;
pub mod validation;

// Re-export main types and functions for external usage
pub use directory::StagingDirectory;
pub use guard::StagingGuard;
pub use manager::StagingManager;
