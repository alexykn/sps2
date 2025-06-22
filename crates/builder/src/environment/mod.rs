//! Build environment management
//!
//! This module provides isolated build environments for package building.
//! It manages directory structure, environment variables, dependency installation,
//! command execution, and environment isolation verification.

mod core;
mod dependencies;
mod directories;
mod execution;
mod hermetic;
mod isolation;
mod types;
mod variables;

// Re-export public API
pub use core::BuildEnvironment;
pub use types::{BuildCommandResult, BuildResult, IsolationLevel};
