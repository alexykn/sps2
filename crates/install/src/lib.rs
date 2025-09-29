#![warn(clippy::pedantic)]
#![deny(clippy::all)]

//! Package installation with atomic updates for sps2
//!
//! This crate handles the installation of packages with atomic
//! state transitions, rollback capabilities, and parallel execution.

#[macro_use]
mod macros;
mod api;
mod atomic;
mod installer;
mod operations;
mod prepare;
//mod pipeline;
//pub mod validation;

pub use atomic::{AtomicInstaller, StateTransition};
pub use installer::Installer;
pub use operations::{InstallOperation, UninstallOperation, UpdateOperation};
pub use prepare::{ExecutionContext, ParallelExecutor};

// Re-export the public API surface from api module
pub use api::config::{InstallConfig, SecurityPolicy};
pub use api::context::{InstallContext, UninstallContext, UpdateContext};
pub use api::result::{InstallResult, StateInfo};
pub use api::types::PreparedPackage;

// Re-export EventSender for use by macros and contexts
pub use sps2_events::EventSender;
