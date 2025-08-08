//! YAML recipe parser with validation and variable expansion

use super::model::{Build, ParsedStep, PostCommand, PostOption, YamlRecipe};
use sps2_errors::{BuildError, Error};
use std::collections::HashMap;
use std::path::Path;

/// Parse a YAML recipe from a file
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The YAML is invalid
/// - Required fields are missing
/// - Validation fails
pub async fn parse_yaml_recipe(path: &Path) -> Result<YamlRecipe, Error> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| BuildError::RecipeError {
            message: format!("failed to read recipe: {e}"),
        })?;

    parse_yaml_recipe_from_string(&content)
}

/// Parse a YAML recipe from a string
///
/// # Errors
///
/// Returns an error if:
/// - The YAML is invalid
/// - Required fields are missing
/// - Validation fails
pub fn parse_yaml_recipe_from_string(content: &str) -> Result<YamlRecipe, Error> {
    let mut recipe: YamlRecipe =
        serde_yml::from_str(content).map_err(|e| BuildError::RecipeError {
            message: format!("failed to parse YAML: {e}"),
        })?;

    // Validate the recipe
    validate_recipe(&recipe)?;

    // Expand variables in the recipe
    expand_variables(&mut recipe);

    Ok(recipe)
}

/// Validate a parsed recipe
fn validate_recipe(recipe: &YamlRecipe) -> Result<(), Error> {
    // Validate metadata
    if recipe.metadata.name.is_empty() {
        return Err(BuildError::RecipeError {
            message: "metadata.name cannot be empty".to_string(),
        }
        .into());
    }

    if recipe.metadata.version.is_empty() {
        return Err(BuildError::RecipeError {
            message: "metadata.version cannot be empty".to_string(),
        }
        .into());
    }

    // Validate build stage
    match &recipe.build {
        Build::System { system, args: _ } => {
            // System builds are always valid
            let _ = system; // Use to avoid unused warning
        }
        Build::Steps { steps } => {
            if steps.is_empty() {
                return Err(BuildError::RecipeError {
                    message: "build.steps cannot be empty".to_string(),
                }
                .into());
            }
        }
    }

    Ok(())
}

/// Expand variables in the recipe using facts and built-in variables
fn expand_variables(recipe: &mut YamlRecipe) {
    // Build variable context
    let mut context = HashMap::new();

    // Add built-in variables
    context.insert("NAME".to_string(), recipe.metadata.name.clone());
    context.insert("VERSION".to_string(), recipe.metadata.version.clone());
    context.insert("PREFIX".to_string(), sps2_config::fixed_paths::LIVE_DIR.to_string());
    context.insert("JOBS".to_string(), num_cpus::get().to_string());

    // Add user-defined facts
    for (key, value) in &recipe.facts {
        context.insert(key.clone(), value.clone());
    }

    // Add environment variables (they can reference facts)
    let mut env_vars = recipe.environment.variables.clone();
    for value in env_vars.values_mut() {
        *value = expand_string(value, &context);
    }
    recipe.environment.variables = env_vars;

    // Update context with expanded environment variables
    for (key, value) in &recipe.environment.variables {
        context.insert(key.clone(), value.clone());
    }

    // Expand variables in build steps
    match &mut recipe.build {
        Build::System { system: _, args } => {
            for arg in args {
                *arg = expand_string(arg, &context);
            }
        }
        Build::Steps { steps } => {
            for step in steps {
                expand_build_step(step, &context);
            }
        }
    }

    // Expand variables in post commands
    for cmd in &mut recipe.post.commands {
        match cmd {
            PostCommand::Simple(s) => *s = expand_string(s, &context),
            PostCommand::Shell { shell } => *shell = expand_string(shell, &context),
        }
    }

    // Expand variables in post option paths
    if let PostOption::Paths(paths) = &mut recipe.post.fix_permissions {
        for path in paths {
            *path = expand_string(path, &context);
        }
    }
}

/// Expand variables in a single string
fn expand_string(input: &str, context: &HashMap<String, String>) -> String {
    let mut result = input.to_string();

    // Expand ${VAR} style variables
    for (key, value) in context {
        result = result.replace(&format!("${{{key}}}"), value);
    }

    // Expand $VAR style variables (but only if followed by non-alphanumeric)
    for (key, value) in context {
        // This is a simple implementation - a more robust one would use regex
        result = result.replace(&format!("${key} "), &format!("{value} "));
        result = result.replace(&format!("${key}/"), &format!("{value}/"));
        result = result.replace(&format!("${key}"), value);
    }

    result
}

/// Expand variables in a build step
fn expand_build_step(step: &mut ParsedStep, context: &HashMap<String, String>) {
    match step {
        ParsedStep::Command { command } => {
            *command = expand_string(command, context);
        }
        ParsedStep::Shell { shell } => {
            *shell = expand_string(shell, context);
        }
        ParsedStep::Make { make } => {
            for arg in make {
                *arg = expand_string(arg, context);
            }
        }
        ParsedStep::Configure { configure } => {
            for arg in configure {
                *arg = expand_string(arg, context);
            }
        }
        ParsedStep::Cmake { cmake } => {
            for arg in cmake {
                *arg = expand_string(arg, context);
            }
        }
        ParsedStep::Meson { meson } => {
            for arg in meson {
                *arg = expand_string(arg, context);
            }
        }
        ParsedStep::Cargo { cargo } => {
            for arg in cargo {
                *arg = expand_string(arg, context);
            }
        }
        ParsedStep::Go { go } => {
            for arg in go {
                *arg = expand_string(arg, context);
            }
        }
        ParsedStep::Python { python } => {
            for arg in python {
                *arg = expand_string(arg, context);
            }
        }
        ParsedStep::Nodejs { nodejs } => {
            for arg in nodejs {
                *arg = expand_string(arg, context);
            }
        }
    }
}
