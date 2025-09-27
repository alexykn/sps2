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

mod atomic;
mod common;
mod installer;
mod operations;
mod parallel;
mod pipeline;
mod staging;
pub mod validation;

pub use atomic::{AtomicInstaller, StateTransition};
pub use installer::{InstallConfig, Installer};
pub use operations::{InstallOperation, UninstallOperation, UpdateOperation};
pub use parallel::SecurityPolicy;
pub use parallel::{ExecutionContext, ParallelExecutor};
pub use pipeline::batch::{BatchResult, BatchStats};
pub use pipeline::config::PipelineConfig;
pub use pipeline::PipelineMaster;
// Note: Python package handling has been moved to builder-centric approach
// The installer now treats Python packages like regular file packages
pub use staging::{StagingDirectory, StagingGuard, StagingManager};
pub use validation::{
    validate_sp_file, validate_tar_archive_content, PackageFormat, ValidationResult,
};

// Removed unused imports: Error, EventSender, ResolutionResult, Version, HashMap
// These will be imported where needed in future implementations
use sps2_hash::Hash;
use sps2_resolver::PackageId;
use sps2_types::PackageSpec;
use std::path::PathBuf;
use uuid::Uuid;

// Re-export EventSender for use by macros
pub use sps2_events::EventSender;

// PreparedPackage will be exported by the pub struct declaration below

/// Installation context
#[derive(Clone, Debug)]
pub struct InstallContext {
    /// Package specifications to install
    pub packages: Vec<PackageSpec>,
    /// Local package files to install
    pub local_files: Vec<PathBuf>,
    /// Force reinstallation
    pub force: bool,

    /// Force re-download even if cached in the store
    pub force_download: bool,

    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

context_builder! {
    InstallContext {
        packages: Vec<PackageSpec>,
        local_files: Vec<PathBuf>,
        force: bool,
        force_download: bool,

    }
}
context_add_package_method!(InstallContext, PackageSpec);

/// Installation result
#[derive(Debug)]
pub struct InstallResult {
    /// State ID after installation
    pub state_id: Uuid,
    /// Packages that were installed
    pub installed_packages: Vec<PackageId>,
    /// Packages that were updated
    pub updated_packages: Vec<PackageId>,
    /// Packages that were removed
    pub removed_packages: Vec<PackageId>,
}

impl InstallResult {
    /// Create new install result
    #[must_use]
    pub fn new(state_id: Uuid) -> Self {
        Self {
            state_id,
            installed_packages: Vec::new(),
            updated_packages: Vec::new(),
            removed_packages: Vec::new(),
        }
    }

    /// Add installed package
    pub fn add_installed(&mut self, package_id: PackageId) {
        self.installed_packages.push(package_id);
    }

    /// Add updated package
    pub fn add_updated(&mut self, package_id: PackageId) {
        self.updated_packages.push(package_id);
    }

    /// Add removed package
    pub fn add_removed(&mut self, package_id: PackageId) {
        self.removed_packages.push(package_id);
    }

    /// Get total number of changes
    #[must_use]
    pub fn total_changes(&self) -> usize {
        self.installed_packages.len() + self.updated_packages.len() + self.removed_packages.len()
    }
}

/// Uninstall context
#[derive(Clone, Debug)]
pub struct UninstallContext {
    /// Package names to uninstall
    pub packages: Vec<String>,
    /// Remove dependencies if no longer needed
    pub autoremove: bool,
    /// Force removal even with dependents
    pub force: bool,

    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

context_builder! {
    UninstallContext {
        packages: Vec<String>,
        autoremove: bool,
        force: bool,

    }
}
context_add_package_method!(UninstallContext, String);

/// Update context
#[derive(Clone, Debug)]
pub struct UpdateContext {
    /// Packages to update (empty = all)
    pub packages: Vec<String>,
    /// Upgrade mode (ignore upper bounds)
    pub upgrade: bool,

    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

context_builder! {
    UpdateContext {
        packages: Vec<String>,
        upgrade: bool,

    }
}
context_add_package_method!(UpdateContext, String);

/// Prepared package data passed from ParallelExecutor to AtomicInstaller
///
/// This structure contains all the information needed by AtomicInstaller
/// to install a package without having to look up package_map or perform
/// additional database queries.
#[derive(Clone, Debug)]
pub struct PreparedPackage {
    /// Package hash
    pub hash: Hash,
    /// Package size in bytes
    pub size: u64,
    /// Path to the package in the store
    pub store_path: PathBuf,
    /// Whether this package was downloaded or local
    pub is_local: bool,
    /// Optional package archive hash (BLAKE3) provided by the repository
    pub package_hash: Option<Hash>,
}
