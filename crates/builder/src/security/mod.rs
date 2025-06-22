//! Build security context and validation
//!
//! This module provides a comprehensive security framework for tracking and
//! validating all file system operations and command executions during builds.

mod context;
mod parser;
mod path_resolver;

pub use context::SecurityContext;
pub use parser::parse_command_with_context;
