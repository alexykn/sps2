#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Package building with SBOM generation for sps2
//!
//! This crate handles building packages from Starlark recipes with
//! isolated environments, dependency management, and SBOM generation.

mod api;
mod archive;
mod builder;
mod compression;
mod config;
mod environment;
mod events;
mod fileops;
mod format;
mod manifest;
mod packaging;
mod quality;
mod recipe;
mod sbom;
mod signing;
mod starlark_bridge;
mod timeout_utils;
mod workflow;

pub use api::BuilderApi;
pub use builder::{BuildConfig, Builder};
pub use compression::{CompressionConfig, CompressionLevel};
pub use environment::{BuildCommandResult, BuildEnvironment, BuildResult};
pub use format::{detect_compression_format, CompressionFormatInfo};
pub use sbom::{SbomConfig, SbomFiles, SbomGenerator};
pub use signing::{PackageSigner, SigningConfig};
pub use starlark_bridge::StarlarkBridge;

// Re-export the compression function for external use
pub use compression::compress_with_zstd;
// Re-export archive functions for external use
pub use archive::{create_deterministic_tar_archive, get_deterministic_timestamp};

use sps2_events::EventSender;
use sps2_types::Version;
use std::path::PathBuf;

/// Build context for package building
#[derive(Clone, Debug)]
pub struct BuildContext {
    /// Package name
    pub name: String,
    /// Package version
    pub version: Version,
    /// Revision number
    pub revision: u32,
    /// Target architecture
    pub arch: String,
    /// Recipe file path
    pub recipe_path: PathBuf,
    /// Output directory for .sp files
    pub output_dir: PathBuf,
    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

impl BuildContext {
    /// Create new build context
    #[must_use]
    pub fn new(name: String, version: Version, recipe_path: PathBuf, output_dir: PathBuf) -> Self {
        Self {
            name,
            version,
            revision: 1,
            arch: "arm64".to_string(),
            recipe_path,
            output_dir,
            event_sender: None,
        }
    }

    /// Set revision number
    #[must_use]
    pub fn with_revision(mut self, revision: u32) -> Self {
        self.revision = revision;
        self
    }

    /// Set architecture
    #[must_use]
    pub fn with_arch(mut self, arch: String) -> Self {
        self.arch = arch;
        self
    }

    /// Set event sender
    #[must_use]
    pub fn with_event_sender(mut self, event_sender: EventSender) -> Self {
        self.event_sender = Some(event_sender);
        self
    }

    /// Get package filename
    #[must_use]
    pub fn package_filename(&self) -> String {
        format!(
            "{}-{}-{}.{}.sp",
            self.name, self.version, self.revision, self.arch
        )
    }

    /// Get full output path
    #[must_use]
    pub fn output_path(&self) -> PathBuf {
        self.output_dir.join(self.package_filename())
    }
}
