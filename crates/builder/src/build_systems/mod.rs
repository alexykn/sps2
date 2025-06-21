//! Build system abstraction and implementations
//!
//! This module provides a trait-based abstraction for different build systems
//! (autotools, cmake, meson, cargo, etc.) with automatic detection and
//! sophisticated configuration handling.

use async_trait::async_trait;
use sps2_errors::Error;
use std::collections::HashMap;
use std::path::Path;

mod autotools;
mod cargo;
mod cmake;
mod core;
mod go;
mod meson;
mod nodejs;
mod python;

pub use autotools::AutotoolsBuildSystem;
pub use cargo::CargoBuildSystem;
pub use cmake::CMakeBuildSystem;
pub use core::{BuildSystemConfig, BuildSystemContext, TestFailure, TestResults};
pub use go::GoBuildSystem;
pub use meson::MesonBuildSystem;
pub use nodejs::NodeJsBuildSystem;
pub use python::PythonBuildSystem;

/// Trait for build system implementations
#[async_trait]
pub trait BuildSystem: Send + Sync {
    /// Detect if this build system applies to the source directory
    async fn detect(&self, source_dir: &Path) -> Result<bool, Error>;

    /// Get configuration options specific to this build system
    fn get_config_options(&self) -> BuildSystemConfig;

    /// Configure phase
    async fn configure(&self, ctx: &BuildSystemContext, args: &[String]) -> Result<(), Error>;

    /// Build phase
    async fn build(&self, ctx: &BuildSystemContext, args: &[String]) -> Result<(), Error>;

    /// Test phase
    async fn test(&self, ctx: &BuildSystemContext) -> Result<TestResults, Error>;

    /// Install phase
    async fn install(&self, ctx: &BuildSystemContext) -> Result<(), Error>;

    /// Get build system specific environment variables
    fn get_env_vars(&self, ctx: &BuildSystemContext) -> HashMap<String, String>;

    /// Get build system name
    fn name(&self) -> &'static str;

    /// Check if out-of-source build is preferred
    fn prefers_out_of_source_build(&self) -> bool {
        false
    }

    /// Get build directory name for out-of-source builds
    fn build_directory_name(&self) -> &'static str {
        "build"
    }
}

/// Registry of available build systems
pub struct BuildSystemRegistry {
    systems: Vec<Box<dyn BuildSystem>>,
}

impl BuildSystemRegistry {
    /// Create a new registry with all supported build systems
    #[must_use]
    pub fn new() -> Self {
        Self {
            systems: vec![
                Box::new(AutotoolsBuildSystem::new()),
                Box::new(CMakeBuildSystem::new()),
                Box::new(MesonBuildSystem::new()),
                Box::new(CargoBuildSystem::new()),
                Box::new(GoBuildSystem::new()),
                Box::new(PythonBuildSystem::new()),
                Box::new(NodeJsBuildSystem::new()),
            ],
        }
    }

    /// Detect which build system to use for a source directory
    ///
    /// # Errors
    ///
    /// Returns an error if detection fails or no suitable build system is found
    pub async fn detect(&self, source_dir: &Path) -> Result<&dyn BuildSystem, Error> {
        for system in &self.systems {
            if system.detect(source_dir).await? {
                return Ok(system.as_ref());
            }
        }

        Err(sps2_errors::BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
        .into())
    }

    /// Get a specific build system by name
    pub fn get(&self, name: &str) -> Option<&dyn BuildSystem> {
        self.systems
            .iter()
            .find(|s| s.name().eq_ignore_ascii_case(name))
            .map(std::convert::AsRef::as_ref)
    }
}

impl Default for BuildSystemRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Automatically detect and return the appropriate build system
///
/// # Errors
///
/// Returns an error if no suitable build system can be detected
pub async fn detect_build_system(source_dir: &Path) -> Result<Box<dyn BuildSystem>, Error> {
    let registry = BuildSystemRegistry::new();
    let system = registry.detect(source_dir).await?;

    // Return a boxed clone of the detected system
    match system.name() {
        "autotools" => Ok(Box::new(AutotoolsBuildSystem::new())),
        "cmake" => Ok(Box::new(CMakeBuildSystem::new())),
        "meson" => Ok(Box::new(MesonBuildSystem::new())),
        "cargo" => Ok(Box::new(CargoBuildSystem::new())),
        "go" => Ok(Box::new(GoBuildSystem::new())),
        "python" => Ok(Box::new(PythonBuildSystem::new())),
        "nodejs" => Ok(Box::new(NodeJsBuildSystem::new())),
        _ => unreachable!("Unknown build system"),
    }
}
