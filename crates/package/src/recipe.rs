//! Recipe data structures

use serde::{Deserialize, Serialize};
use sps2_errors::{BuildError, Error};

/// A build recipe
#[derive(Debug, Clone)]
pub struct Recipe {
    pub content: String,
    pub path: Option<String>,
}

impl Recipe {
    /// Parse recipe from content
    ///
    /// # Errors
    ///
    /// Returns a `BuildError::RecipeError` if the recipe content is missing
    /// required `metadata` or `build` functions.
    pub fn parse(content: &str) -> Result<Self, Error> {
        // Basic validation - check for required functions
        if !content.contains("def metadata") {
            return Err(BuildError::RecipeError {
                message: "recipe missing required 'metadata' function".to_string(),
            }
            .into());
        }

        if !content.contains("def build") {
            return Err(BuildError::RecipeError {
                message: "recipe missing required 'build' function".to_string(),
            }
            .into());
        }

        Ok(Self {
            content: content.to_string(),
            path: None,
        })
    }

    /// Set the recipe path (for error reporting)
    #[must_use]
    pub fn with_path(mut self, path: String) -> Self {
        self.path = Some(path);
        self
    }
}

/// Recipe metadata collected from `metadata()` function
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecipeMetadata {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub runtime_deps: Vec<String>,
    pub build_deps: Vec<String>,
}

/// A build step from the `build()` function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BuildStep {
    Fetch { url: String, blake3: String },
    ApplyPatch { path: String },
    AllowNetwork { enabled: bool },
    Configure { args: Vec<String> },
    Make { args: Vec<String> },
    Autotools { args: Vec<String> },
    Cmake { args: Vec<String> },
    Meson { args: Vec<String> },
    Cargo { args: Vec<String> },
    Command { program: String, args: Vec<String> },
    SetEnv { key: String, value: String },
    Install,
}
