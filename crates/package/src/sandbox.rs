//! Sandboxed Starlark execution environment

use crate::recipe::{BuildStep, Recipe, RecipeMetadata};
use crate::starlark_api::{build_api, parse_metadata, BuildContext};
use spsv2_errors::{BuildError, Error};
use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect};

/// Result of recipe execution
#[derive(Debug)]
pub struct RecipeResult {
    pub metadata: RecipeMetadata,
    pub build_steps: Vec<BuildStep>,
}

/// Sandboxed recipe execution engine
pub struct RecipeEngine {
    globals: Globals,
}

impl RecipeEngine {
    /// Create a new sandboxed engine
    pub fn new() -> Self {
        let globals = GlobalsBuilder::standard()
            .with(build_api)
            .build();
        
        Self { globals }
    }

    /// Execute a recipe
    pub fn execute(&self, recipe: &Recipe) -> Result<RecipeResult, Error> {
        // Parse the recipe
        let ast = AstModule::parse(
            &recipe.path.as_deref().unwrap_or("recipe.star"),
            recipe.content.clone(),
            &Dialect::Standard,
        )
        .map_err(|e| BuildError::RecipeError {
            message: format!("Failed to parse recipe: {}", e),
        })?;

        // Create module and evaluator
        let module = Module::new();
        let mut eval = Evaluator::new(&module);

        // Set resource limits
        let _ = eval.set_max_callstack_size(100);
        
        // Evaluate the recipe
        eval.eval_module(ast, &self.globals)
            .map_err(|e| BuildError::RecipeError {
                message: format!("Recipe evaluation failed: {}", e),
            })?;

        // Drop the evaluator before freezing
        drop(eval);

        // Freeze the module to access functions
        let frozen_module = module.freeze()
            .map_err(|e| BuildError::RecipeError {
                message: format!("Failed to freeze module: {}", e),
            })?;

        // Call metadata function
        let metadata_fn = frozen_module
            .get("metadata")
            .map_err(|e| BuildError::RecipeError {
                message: format!("Failed to get metadata function: {}", e),
            })?;

        let metadata_module = Module::new();
        let mut metadata_eval = Evaluator::new(&metadata_module);
        
        let metadata_value = metadata_eval
            .eval_function(metadata_fn.value(), &[], &[])
            .map_err(|e| BuildError::RecipeError {
                message: format!("Failed to execute metadata(): {}", e),
            })?;

        let metadata = parse_metadata(metadata_value)?;

        // Call build function with context
        let build_fn = frozen_module
            .get("build")
            .map_err(|e| BuildError::RecipeError {
                message: format!("Failed to get build function: {}", e),
            })?;

        let build_module = Module::new();
        let mut build_eval = Evaluator::new(&build_module);

        // Create build context
        let context = BuildContext::new(
            "/opt/pm/build".to_string(), // This will be replaced by builder
            num_cpus::get() as i32,
        );

        // Allocate the context in the build module's heap
        let context_value = build_module.heap().alloc(context.clone());
        
        build_eval
            .eval_function(build_fn.value(), &[context_value], &[])
            .map_err(|e| BuildError::RecipeError {
                message: format!("Failed to execute build(): {}", e),
            })?;

        // Extract build steps
        let build_steps = context.steps.into_inner();

        // Validate results
        if metadata.name.is_empty() {
            return Err(BuildError::RecipeError {
                message: "Package name not set in metadata()".to_string(),
            }
            .into());
        }

        if metadata.version.is_empty() {
            return Err(BuildError::RecipeError {
                message: "Package version not set in metadata()".to_string(),
            }
            .into());
        }

        if !build_steps.iter().any(|s| matches!(s, BuildStep::Install)) {
            return Err(BuildError::RecipeError {
                message: "build() function must call ctx.install()".to_string(),
            }
            .into());
        }

        Ok(RecipeResult {
            metadata,
            build_steps,
        })
    }
}

impl Default for RecipeEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_recipe() {
        let recipe_content = r#"
def metadata():
    return struct(
        name = "test-pkg",
        version = "1.0.0",
        description = "Test package"
    )

def build(ctx):
    ctx.fetch("https://example.com/src.tar.gz", "abc123")
    ctx.make([])
    ctx.install()
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe).unwrap();

        assert_eq!(result.metadata.name, "test-pkg");
        assert_eq!(result.metadata.version, "1.0.0");
        assert_eq!(result.build_steps.len(), 3);
    }
}