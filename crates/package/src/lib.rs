#![deny(clippy::pedantic)]
#![allow(unsafe_code)] // Required for Starlark trait implementations
#![allow(clippy::module_name_repetitions)]

//! Starlark recipe handling for sps2
//!
//! This crate provides the sandboxed Starlark environment for build recipes,
//! exposing a limited API for package metadata and build operations.

mod error_helpers;
mod recipe;
mod sandbox;
mod starlark_api;

pub use recipe::{BuildStep, Recipe, RecipeMetadata};
pub use sandbox::{RecipeEngine, RecipeResult};
pub use starlark_api::BuildContext;

use sps2_errors::Error;
use std::path::Path;

/// Load and parse a recipe file
///
/// # Errors
///
/// Returns a `BuildError::RecipeError` if the file cannot be read from the filesystem
/// or if the recipe content is invalid (missing required functions).
pub async fn load_recipe(path: &Path) -> Result<Recipe, Error> {
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        sps2_errors::BuildError::RecipeError {
            message: format!("failed to read recipe: {e}"),
        }
    })?;

    Recipe::parse(&content)
}

/// Execute a recipe and get metadata
///
/// # Errors
///
/// Returns an error if the recipe execution fails, including:
/// - Starlark parsing or evaluation errors
/// - Missing or invalid metadata in the recipe
/// - Runtime errors during recipe execution
pub fn execute_recipe(recipe: &Recipe) -> Result<RecipeResult, Error> {
    let engine = RecipeEngine::new();
    engine.execute(recipe)
}
