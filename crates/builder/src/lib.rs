#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]
// Development-time allowances for incomplete features
// The builder crate is under heavy development with many placeholder implementations
#![allow(clippy::missing_docs_in_private_items)] // TODO: Add private docs
#![allow(clippy::must_use_candidate)] // TODO: Add must_use attributes systematically
#![allow(clippy::unused_async)] // Some async fns are prepared for future async work
#![allow(clippy::redundant_closure)] // Some closures improve readability
#![allow(clippy::match_same_arms)] // Placeholder implementations during development
#![allow(clippy::missing_panics_doc)] // TODO: Add comprehensive panic docs
#![allow(clippy::doc_markdown)] // TODO: Fix documentation formatting systematically
#![allow(clippy::cast_precision_loss)] // Acceptable for statistics and progress tracking
#![allow(clippy::redundant_clone)] // Some clones improve code clarity during development
#![allow(clippy::return_self_not_must_use)] // Builder patterns don't require must_use in dev
#![allow(clippy::single_match_else)] // Some matches are clearer than if-else during dev
#![allow(clippy::default_trait_access)] // Default::default() is clear in context
#![allow(clippy::if_not_else)] // Some patterns are clearer with if-not structure
#![allow(clippy::implicit_clone)] // Explicit clones preferred during development
#![allow(clippy::wildcard_imports)] // Acceptable for internal modules during dev
#![allow(clippy::case_sensitive_file_extension_comparisons)] // Simple string checks are fine
#![allow(clippy::map_unwrap_or)] // Some patterns are clearer than unwrap_or_else during dev
#![allow(clippy::cast_possible_truncation)] // Acceptable for metrics and statistics
#![allow(clippy::cast_sign_loss)] // Acceptable for metrics conversion
#![allow(clippy::format_push_string)] // String formatting patterns during development
#![allow(clippy::unnecessary_wraps)] // Some Result wraps are planned for future error cases
#![allow(clippy::needless_return)] // Explicit returns improve clarity during development
#![allow(clippy::manual_let_else)] // Traditional if-let patterns are clearer during dev
#![allow(clippy::unnecessary_lazy_evaluations)] // Some patterns improve clarity
#![allow(clippy::uninlined_format_args)] // Format args can be more readable when separate
#![allow(clippy::iter_on_single_items)] // Some patterns prepared for multi-item iteration
#![allow(clippy::unchecked_duration_subtraction)] // Acceptable for timing metrics

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
