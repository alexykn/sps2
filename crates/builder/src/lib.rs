#![deny(clippy::pedantic, unsafe_code)]
//! Package building with SBOM generation for sps2
//!
//! This crate handles building packages from YAML recipes with
//! isolated environments, dependency management, and SBOM generation.

mod build_plan;
mod build_systems;
mod cache;
mod core;
mod environment;
mod packaging;
mod recipe;
mod utils;
pub mod validation;
mod yaml;

pub use build_systems::{
    detect_build_system, AutotoolsBuildSystem, BuildSystem, BuildSystemConfig, BuildSystemContext,
    BuildSystemRegistry, CMakeBuildSystem, CargoBuildSystem, GoBuildSystem, MesonBuildSystem,
    NodeJsBuildSystem, PythonBuildSystem, TestFailure, TestResults,
};
pub use cache::{
    Artifact, ArtifactType, BuildCache, BuildInputs, CacheKey, CacheStatistics, CompilerCache,
    CompilerCacheType, IncrementalBuildTracker,
};
pub use core::api::BuilderApi;
pub use core::builder::Builder;
pub use core::config::BuildConfig;
pub use environment::{BuildCommandResult, BuildEnvironment, BuildResult};
pub use utils::format::{detect_compression_format, CompressionFormatInfo};

// Re-export packaging types
pub use packaging::archive::{create_deterministic_tar_archive, get_deterministic_timestamp};
pub use packaging::compression::{compress_with_zstd, CompressionConfig, CompressionLevel};
pub use packaging::sbom::{SbomConfig, SbomFiles, SbomGenerator};
pub use packaging::signing::{PackageSigner, SigningConfig};
// Re-export YAML types (from yaml module)
pub use yaml::{BuildStep, RecipeMetadata};

// Re-export recipe types (from recipe module)
pub use recipe::model::{
    Build, BuildStep as YamlBuildStep, BuildSystem as YamlBuildSystem, ChecksumAlgorithm,
    PostCommand, PostOption, SourceMethod, YamlRecipe,
};
pub use recipe::parser::parse_yaml_recipe;

pub use core::context::BuildContext;
