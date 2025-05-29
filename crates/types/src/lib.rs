#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Core type definitions for the spsv2 package manager
//!
//! This crate provides fundamental types used throughout the system,
//! including version specifications, package information, and common data structures.

pub mod package;
pub mod state;
pub mod version;

// Re-export commonly used types
pub use package::{DepKind, PackageId, PackageInfo, PackageSpec, SearchResult};
pub use semver::Version;
pub use state::{StateId, StateInfo};
pub use uuid::Uuid;
pub use version::{VersionConstraint, VersionSpec};

use serde::{Deserialize, Serialize};

/// Architecture type for packages
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arch {
    #[serde(rename = "arm64")]
    Arm64,
}

impl std::fmt::Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arm64 => write!(f, "arm64"),
        }
    }
}

/// Output format for CLI commands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Plain,
    Tty,
    Json,
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Tty
    }
}

/// Color output choice
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorChoice {
    Always,
    Auto,
    Never,
}

// Implement clap::ValueEnum for ColorChoice
impl clap::ValueEnum for ColorChoice {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Always, Self::Auto, Self::Never]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(match self {
            Self::Always => clap::builder::PossibleValue::new("always"),
            Self::Auto => clap::builder::PossibleValue::new("auto"),
            Self::Never => clap::builder::PossibleValue::new("never"),
        })
    }
}

impl Default for ColorChoice {
    fn default() -> Self {
        Self::Auto
    }
}
