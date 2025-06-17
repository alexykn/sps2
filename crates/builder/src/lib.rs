#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]
// Development-time allowances for incomplete features
// The builder crate is under heavy development with many placeholder implementations
#![allow(dead_code)] // Many functions are planned for future use
#![allow(clippy::unused_self)] // Methods designed for polymorphic use
#![allow(clippy::missing_errors_doc)] // TODO: Add comprehensive error docs
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
//! This crate handles building packages from Starlark recipes with
//! isolated environments, dependency management, and SBOM generation.

/// Placeholder prefix used during builds to enable relocatable packages
/// This gets replaced with the actual installation prefix during packaging
pub const BUILD_PLACEHOLDER_PREFIX: &str = "/SPS2_PLACEHOLDER_PREFIX_REPLACE_ME";

mod api;
mod archive;
mod build_systems;
mod builder;
mod cache;
mod compression;
mod config;
mod cross;
pub mod dependencies;
mod environment;
pub mod error_handling;
mod events;
mod fileops;
mod format;
mod manifest;
mod monitoring;
mod orchestration;
mod packaging;
pub mod post_validation;
pub mod quality_assurance;
mod recipe;
mod sbom;
mod signing;
mod starlark_bridge;
mod timeout_utils;
mod workflow;

pub use api::BuilderApi;
pub use build_systems::{
    detect_build_system, AutotoolsBuildSystem, BuildSystem, BuildSystemConfig, BuildSystemContext,
    BuildSystemRegistry, CMakeBuildSystem, CargoBuildSystem, CrossCompilationContext,
    GoBuildSystem, MesonBuildSystem, NodeJsBuildSystem, Platform, PythonBuildSystem, TestFailure,
    TestResults, Toolchain,
};
pub use builder::Builder;
pub use cache::{
    Artifact, ArtifactType, BuildCache, BuildInputs, CacheKey, CacheStatistics, CompilerCache,
    CompilerCacheType, IncrementalBuildTracker,
};
pub use compression::{CompressionConfig, CompressionLevel};
pub use config::BuildConfig;
pub use dependencies::{
    Dependency, DependencyContext, DependencyGraph, DependencyNode, DependencyResolver,
    ExtendedDepKind,
};
pub use environment::{BuildCommandResult, BuildEnvironment, BuildResult};
pub use format::{detect_compression_format, CompressionFormatInfo};
pub use monitoring::{
    BuildMonitor, BuildSpan, Metric, MetricType, MetricsAggregator, MetricsCollector,
    MetricsSnapshot, MonitoringConfig, MonitoringLevel, MonitoringPipeline, MonitoringSystem,
    ResourceMetrics, SpanContext, StatisticalSummary, TelemetryCollector, TracingCollector,
};
pub use orchestration::{
    BuildOrchestrator, BuildScheduler, BuildTask, OrchestratorStats, Priority, ResourceManager,
    ResourceRequirements, SchedulerStats, SystemResources, TaskState,
};
pub use sbom::{SbomConfig, SbomFiles, SbomGenerator};
pub use signing::{PackageSigner, SigningConfig};
pub use starlark_bridge::StarlarkBridge;

// Re-export the compression function for external use
pub use compression::compress_with_zstd;
pub use cross::EnhancedCrossContext;
// Re-export archive functions for external use
pub use archive::{create_deterministic_tar_archive, get_deterministic_timestamp};
// Re-export error handling components
pub use error_handling::{
    with_recovery, BuildCheckpoint, BuildErrorHandler, BuildState, CheckpointManager,
    CompilationFailedRecovery, DependencyConflictRecovery, DiskSpaceRecovery, NetworkErrorRecovery,
    RecoveryAction, RecoveryStrategy, TestsFailedRecovery,
};

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
    /// Path to the generated .sp package (set after package creation)
    pub package_path: Option<PathBuf>,
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
            package_path: None,
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
