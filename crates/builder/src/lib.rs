#![deny(clippy::pedantic, unsafe_code)]
//! Package building with SBOM generation for sps2
//!
//! This crate handles building packages from YAML recipes with
//! isolated environments, dependency management, and SBOM generation.

pub mod artifact_qa;
mod build_plan;
mod build_systems;
mod cache;
mod core;
mod environment;
mod packaging;
mod recipe;
mod security;
mod stages;
mod utils;
mod validation;
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
pub use packaging::manifest::generate_sbom_and_manifest;
pub use packaging::sbom::{SbomConfig, SbomFiles, SbomGenerator};
pub use packaging::signing::{PackageSigner, SigningConfig};
pub use packaging::{create_and_sign_package, create_package};
// Re-export YAML types (from yaml module)
pub use yaml::{BuildStep, RecipeMetadata};

// Re-export recipe types (from recipe module)
pub use recipe::model::{
    Build, BuildSystem as YamlBuildSystem, ChecksumAlgorithm, ParsedStep, PostCommand, PostOption,
    RpathPatchOption, SourceMethod, YamlRecipe,
};
pub use recipe::parser::parse_yaml_recipe;

pub use core::context::BuildContext;

// Re-export build plan and security types for pack command
pub use build_plan::BuildPlan;
pub use security::SecurityContext;
pub use stages::build::BuildCommand;
pub use stages::executors::execute_post_step_with_security;
