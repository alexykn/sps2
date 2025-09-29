#![warn(mismatched_lifetime_syntaxes)]
#![deny(clippy::pedantic)]
#![allow(clippy::module_name_repetitions, unsafe_code)]
#![allow(
    clippy::needless_continue,
    clippy::collapsible_else_if,
    clippy::redundant_else
)]
#![allow(
    clippy::missing_errors_doc,
    clippy::single_match_else,
    clippy::too_many_lines
)]
#![allow(
    clippy::doc_markdown,
    clippy::uninlined_format_args,
    clippy::cast_precision_loss
)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::struct_excessive_bools,
    clippy::must_use_candidate
)]
#![allow(
    clippy::single_char_pattern,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
#![allow(
    clippy::if_not_else,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    clippy::match_same_arms
)]
// Additional allows for modularization artifacts - to be cleaned up later
#![allow(
    clippy::unnecessary_map_or,
    clippy::type_complexity,
    clippy::to_string_in_format_args
)]
#![allow(clippy::manual_map, clippy::manual_strip)]

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
