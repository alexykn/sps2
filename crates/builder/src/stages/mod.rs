//! Stage-specific types for the builder
//!
//! This module provides types for each stage of the build process,
//! maintaining a clear separation between parsing and execution.

pub mod build;
pub mod environment;
pub mod executors;
pub mod post;
pub mod source;

// Re-export execution types
pub use build::BuildCommand;
pub use environment::EnvironmentStep;
pub use post::PostStep;
pub use source::SourceStep;

// The executors are used internally by utils/executor.rs
