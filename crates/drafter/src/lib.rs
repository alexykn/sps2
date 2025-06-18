#![deny(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

//! Recipe drafter for sps2
//!
//! This crate provides functionality to analyze source code and generate
//! Starlark build recipes automatically.

mod archive;
mod detector;
mod metadata;
mod source;
mod template;

pub use source::SourceLocation;

use sps2_errors::Error;

/// Type alias for results in this crate
pub type Result<T> = std::result::Result<T, Error>;

use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Result of recipe drafting
#[derive(Debug)]
pub struct DraftResult {
    /// Generated recipe content
    pub recipe_content: String,
    /// Extracted metadata
    pub metadata: RecipeMetadata,
}

/// Recipe metadata
#[derive(Debug, Clone)]
pub struct RecipeMetadata {
    /// Package name
    pub name: String,
    /// Package version
    pub version: String,
    /// Package description
    pub description: Option<String>,
    /// Package homepage
    pub homepage: Option<String>,
    /// Package license
    pub license: Option<String>,
}

/// Main drafter struct
pub struct Drafter {
    source_location: SourceLocation,
    event_tx: Option<sps2_events::EventSender>,
}

impl Drafter {
    /// Create a new drafter
    #[must_use]
    pub fn new(source_location: SourceLocation) -> Self {
        Self {
            source_location,
            event_tx: None,
        }
    }

    /// Set event sender for progress reporting
    #[must_use]
    pub fn with_event_sender(mut self, tx: sps2_events::EventSender) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Run the drafting process
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Source preparation fails
    /// - Metadata extraction fails
    /// - Build system detection fails
    /// - Template rendering fails
    pub async fn run(&self) -> Result<DraftResult> {
        // Send progress event
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(sps2_events::Event::OperationStarted {
                operation: "Starting recipe draft generation".to_string(),
            });
        }

        // Prepare source directory
        let (_temp_dir, source_dir) = self.prepare_source().await?;

        // Extract metadata
        let metadata = self.extract_metadata(&source_dir).await?;

        // Detect build system
        let build_info = self.detect_build_system(&source_dir).await?;

        // Generate recipe
        let recipe_content = self.generate_recipe(&metadata, &build_info, &source_dir)?;

        Ok(DraftResult {
            recipe_content,
            metadata,
        })
    }

    /// Prepare source directory based on source location
    async fn prepare_source(&self) -> Result<(Option<TempDir>, PathBuf)> {
        source::prepare(&self.source_location, self.event_tx.as_ref()).await
    }

    /// Extract metadata from source directory
    async fn extract_metadata(&self, source_dir: &Path) -> Result<RecipeMetadata> {
        metadata::extract_metadata(source_dir).await
    }

    /// Detect build system and extract dependencies
    async fn detect_build_system(&self, source_dir: &Path) -> Result<BuildInfo> {
        detector::detect(source_dir).await
    }

    /// Generate recipe from metadata and build info
    fn generate_recipe(
        &self,
        metadata: &RecipeMetadata,
        build_info: &BuildInfo,
        _source_dir: &Path,
    ) -> Result<String> {
        template::render(metadata, build_info, &self.source_location)
    }
}

/// Build system information
#[derive(Debug)]
pub struct BuildInfo {
    /// Detected build system name
    pub build_system: String,
    /// Build function to use in recipe
    pub build_function: String,
    /// Additional build arguments
    pub build_args: Vec<String>,
    /// Detected dependencies
    pub dependencies: Vec<Dependency>,
    /// Whether network access is needed
    pub needs_network: bool,
}

/// A detected dependency
#[derive(Debug)]
pub struct Dependency {
    /// Original dependency name
    pub original: String,
    /// Mapped sps2 package name
    pub sps2_name: String,
    /// Whether this is a build-time dependency
    pub build_time: bool,
}
