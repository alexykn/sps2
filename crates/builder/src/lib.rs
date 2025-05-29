#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Package building with SBOM generation for spsv2
//!
//! This crate handles building packages from Rhai recipes with
//! isolated environments, dependency management, and SBOM generation.

mod api;
mod builder;
mod environment;
mod sbom;

pub use api::BuilderApi;
pub use builder::{BuildConfig, Builder};
pub use environment::{BuildEnvironment, BuildCommandResult, BuildResult};
pub use sbom::{SbomConfig, SbomFiles, SbomGenerator};

use spsv2_errors::Error;
use spsv2_events::EventSender;
use spsv2_types::Version;
use std::path::{Path, PathBuf};

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
    pub fn with_revision(mut self, revision: u32) -> Self {
        self.revision = revision;
        self
    }

    /// Set architecture
    pub fn with_arch(mut self, arch: String) -> Self {
        self.arch = arch;
        self
    }

    /// Set event sender
    pub fn with_event_sender(mut self, event_sender: EventSender) -> Self {
        self.event_sender = Some(event_sender);
        self
    }

    /// Get package filename
    pub fn package_filename(&self) -> String {
        format!(
            "{}-{}-{}.{}.sp",
            self.name, self.version, self.revision, self.arch
        )
    }

    /// Get full output path
    pub fn output_path(&self) -> PathBuf {
        self.output_dir.join(self.package_filename())
    }
}
