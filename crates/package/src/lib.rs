#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Rhai recipe handling for spsv2
//!
//! This crate provides the sandboxed Rhai environment for build recipes,
//! exposing a limited API for package metadata and build operations.

mod api;
mod recipe;
mod sandbox;

pub use api::{BuilderApi, MetadataApi};
pub use recipe::{BuildStep, Recipe, RecipeMetadata};
pub use sandbox::{RecipeEngine, RecipeResult};

use spsv2_errors::Error;
use std::path::Path;

/// Load and parse a recipe file
pub async fn load_recipe(path: &Path) -> Result<Recipe, Error> {
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        spsv2_errors::BuildError::RecipeError {
            message: format!("failed to read recipe: {e}"),
        }
    })?;

    Recipe::parse(&content)
}

/// Execute a recipe and get metadata
pub fn execute_recipe(recipe: &Recipe) -> Result<RecipeResult, Error> {
    let engine = RecipeEngine::new();
    engine.execute(recipe)
}
