//! YAML recipe handling
//!
//! This module provides YAML-based recipe format for build recipes,
//! using a declarative, staged approach for package building.

mod recipe;
mod yaml_parser;
mod yaml_recipe;

pub use recipe::{BuildStep, RecipeMetadata};
pub use yaml_parser::parse_yaml_recipe;
pub use yaml_recipe::{
    Build, BuildStep as YamlBuildStep, BuildSystem, ChecksumAlgorithm, PostOption, SourceMethod,
    YamlRecipe,
};
