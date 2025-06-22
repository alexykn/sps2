//! YAML recipe handling
//!
//! This module provides YAML-based recipe format for build recipes,
//! using a declarative, staged approach for package building.

mod recipe;

pub use recipe::{BuildStep, RecipeMetadata};
