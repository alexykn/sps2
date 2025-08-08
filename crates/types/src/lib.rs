#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Core type definitions for the sps2 package manager
//!
//! This crate provides fundamental types used throughout the system,
//! including version specifications, package information, and common data structures.

pub mod format;
pub mod package;
pub mod manifest;
pub mod recipe;
pub mod reports;
pub mod state;
pub mod version;

// Re-export commonly used types
pub use format::{
    CompressionFormatType, PackageFormatChecker, PackageFormatCompatibility,
    PackageFormatMigration, PackageFormatValidationResult, PackageFormatVersion,
    PackageFormatVersionError,
};
pub use package::{
    DepEdge, DepKind, PackageId, PackageInfo, PackageSpec, PackageStatus, PythonPackageMetadata,
    SearchResult,
};
pub use manifest::{CompressionInfo as ManifestCompressionInfo, Dependencies as ManifestDependencies, Manifest, ManifestBuilder, PackageInfo as ManifestPackageInfo, SbomInfo};
pub use recipe::{
    Build, BuildSystem, Checksum, ChecksumAlgorithm, Dependencies, Environment, FetchSource,
    GitSource, Install, IsolationLevel, LocalSource, Metadata, NamedSource, ParsedStep, Post,
    PostOption, Source, SourceMethod, YamlRecipe,
};
pub use reports::{BuildReport, InstallReport, PackageChange};
pub use semver::Version;
pub use state::{ChangeType, OpChange, StateId, StateInfo, StateTransition};
pub use uuid::Uuid;
pub use version::{VersionConstraint, VersionSpec};

// QA pipeline override is defined below in this module

use serde::{Deserialize, Serialize};

/// Architecture type for packages
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arch {
    #[serde(rename = "arm64")]
    Arm64,
}

/// `RPath` handling style for dynamic libraries and executables
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RpathStyle {
    /// Modern approach: Keep @rpath references with proper `LC_RPATH` entries
    /// This is the default and recommended approach for relocatable binaries
    Modern,
    /// Absolute approach: Rewrite all @rpath references to absolute paths
    /// Use this for compatibility with tools that don't handle @rpath correctly
    Absolute,
}

impl std::fmt::Display for RpathStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Modern => write!(f, "Modern"),
            Self::Absolute => write!(f, "Absolute"),
        }
    }
}

impl Default for RpathStyle {
    fn default() -> Self {
        Self::Modern
    }
}

/// Build system profile for post-validation pipeline selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildSystemProfile {
    /// C/C++ build systems (autotools, cmake, meson) - full validation pipeline
    /// Includes all validators and patchers, binary patching, and code re-signing
    NativeFull,
    /// Rust build system - minimal validation to avoid breaking panic unwinding
    /// Skips binary patching and code re-signing that interfere with Rust runtime
    RustMinimal,
    /// Go build system - medium validation for mostly static binaries
    /// Limited patching, no rpath needed unless CGO is used
    GoMedium,
    /// Script-based systems (Python, Node.js) - light validation
    /// Focus on permissions and text file patching only
    ScriptLight,
}

/// QA pipeline override for manual recipe control
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QaPipelineOverride {
    /// Use automatic detection based on build systems used
    Auto,
    /// Force Rust minimal pipeline (skip binary patching)
    Rust,
    /// Force C/C++ full pipeline (comprehensive validation and patching)
    C,
    /// Force Go medium pipeline (limited patching)
    Go,
    /// Force Python/script light pipeline (text file patching only)
    Python,
    /// Skip artifact QA entirely (dangerous, use only for special cases)
    Skip,
}

impl Default for QaPipelineOverride {
    fn default() -> Self {
        Self::Auto
    }
}

impl QaPipelineOverride {
    /// Convert override to build system profile
    #[must_use]
    pub fn to_profile(self) -> Option<BuildSystemProfile> {
        match self {
            Self::Auto | Self::Skip => None, // Auto: use detection, Skip: skip QA entirely
            Self::Rust => Some(BuildSystemProfile::RustMinimal),
            Self::C => Some(BuildSystemProfile::NativeFull),
            Self::Go => Some(BuildSystemProfile::GoMedium),
            Self::Python => Some(BuildSystemProfile::ScriptLight),
        }
    }

    /// Check if this override skips QA entirely
    #[must_use]
    pub fn skips_qa(self) -> bool {
        matches!(self, Self::Skip)
    }

    /// Check if this is the default value (for serde `skip_serializing_if`)
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Auto)
    }
}

impl BuildSystemProfile {
    /// Determine profile from build system name
    #[must_use]
    pub fn from_build_system(build_system: &str) -> Self {
        match build_system {
            "cargo" => Self::RustMinimal,
            "go" => Self::GoMedium,
            "python" | "nodejs" => Self::ScriptLight,
            _ => Self::NativeFull, // Default to full validation for unknown systems
        }
    }
}

impl Default for BuildSystemProfile {
    fn default() -> Self {
        Self::NativeFull
    }
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
