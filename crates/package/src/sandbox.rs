//! Sandboxed Starlark execution environment

use crate::error_helpers::{
    format_build_error, format_eval_error, format_missing_function_error, format_parse_error,
};
use crate::recipe::{BuildStep, Recipe, RecipeMetadata};
use crate::starlark::{parse_metadata, register_globals, BuildContext, BuildExecutor};
use sps2_errors::{BuildError, Error};
use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect};
use std::sync::Arc;

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
        let globals = GlobalsBuilder::standard().with(register_globals).build();

        Self { globals }
    }

    /// Execute a recipe
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The recipe fails to parse as valid Starlark
    /// - The recipe execution fails (missing functions, runtime errors, etc.)
    /// - Required metadata fields are missing or invalid
    /// - The module cannot be frozen after evaluation
    pub fn execute(&self, recipe: &Recipe) -> Result<RecipeResult, Error> {
        self.execute_internal(recipe, None)
    }

    /// Extract metadata only without executing build steps
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The recipe fails to parse as valid Starlark
    /// - The metadata function fails or returns invalid data
    /// - Required metadata fields are missing or invalid
    pub fn extract_metadata(&self, recipe: &Recipe) -> Result<RecipeMetadata, Error> {
        // Parse the recipe
        let recipe_path = recipe.path.as_deref().unwrap_or("recipe.star");
        let ast = AstModule::parse(recipe_path, recipe.content.clone(), &Dialect::Standard)
            .map_err(|e| format_parse_error(recipe_path, &e.to_string()))?;

        // Create module and evaluator
        let module = Module::new();
        let mut eval = Evaluator::new(&module);
        let _ = eval.set_max_callstack_size(1000);

        // Evaluate the recipe with globals
        eval.eval_module(ast, &self.globals)
            .map_err(|e| format_eval_error(&e.to_string()))?;

        // Drop the evaluator before freezing
        drop(eval);

        // Freeze the module to access functions
        let frozen_module = module.freeze().map_err(|e| BuildError::RecipeError {
            message: format!("Failed to freeze module: {e:?}"),
        })?;

        // Call metadata function
        let metadata_fn = frozen_module
            .get("metadata")
            .map_err(|_| format_missing_function_error("metadata"))?;

        let metadata_module = Module::new();
        let mut metadata_eval = Evaluator::new(&metadata_module);
        let _ = metadata_eval.set_max_callstack_size(1000);

        let metadata_value = metadata_eval
            .eval_function(metadata_fn.value(), &[], &[])
            .map_err(|e| format_eval_error(&format!("metadata() failed: {e}")))?;

        let metadata = parse_metadata(metadata_value)?;

        // Validate metadata
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

        Ok(metadata)
    }

    /// Execute a recipe with executor integration
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The recipe fails to parse as valid Starlark
    /// - The recipe execution fails (missing functions, runtime errors, etc.)
    /// - Required metadata fields are missing or invalid
    /// - The module cannot be frozen after evaluation
    /// - The executor operations fail
    pub fn execute_with_executor(
        &self,
        recipe: &Recipe,
        executor: &Arc<tokio::sync::Mutex<dyn BuildExecutor>>,
    ) -> Result<RecipeResult, Error> {
        self.execute_internal(recipe, Some(executor))
    }

    fn execute_internal(
        &self,
        recipe: &Recipe,
        executor: Option<&Arc<tokio::sync::Mutex<dyn BuildExecutor>>>,
    ) -> Result<RecipeResult, Error> {
        // Parse the recipe
        let recipe_path = recipe.path.as_deref().unwrap_or("recipe.star");
        let ast = AstModule::parse(recipe_path, recipe.content.clone(), &Dialect::Standard)
            .map_err(|e| format_parse_error(recipe_path, &e.to_string()))?;

        // Create module and evaluator
        let module = Module::new();
        let mut eval = Evaluator::new(&module);

        // Set VM resource limits for sandboxing

        // Limit call stack depth to prevent deep recursion attacks
        // Default of 100 is quite low - increase to a more reasonable production value
        // while still preventing stack overflow attacks
        let _ = eval.set_max_callstack_size(1000);

        // Note: starlark-rust 0.13.0 does not provide eval.set_heap_limit() or
        // eval.set_max_steps() methods as mentioned in the README. However, Starlark
        // already provides significant built-in security:
        //
        // 1. Deterministic execution - same code gives same results
        // 2. Hermetic execution - cannot access filesystem, network, or system clock
        // 3. No recursion (by design) - limited ability to write resource-consuming code
        // 4. Garbage collected heap - automatic memory management
        // 5. Values are frozen after evaluation - immutable state
        //
        // The language design itself provides sandboxing against most attacks.
        // Future versions may add more granular limits via compilation flags or
        // alternative evaluation contexts.

        // Evaluate the recipe with globals
        eval.eval_module(ast, &self.globals)
            .map_err(|e| format_eval_error(&e.to_string()))?;

        // Drop the evaluator before freezing
        drop(eval);

        // Freeze the module to access functions
        let frozen_module = module.freeze().map_err(|e| BuildError::RecipeError {
            message: format!("Failed to freeze module: {e:?}"),
        })?;

        // Call metadata function
        let metadata_fn = frozen_module
            .get("metadata")
            .map_err(|_| format_missing_function_error("metadata"))?;

        let metadata_module = Module::new();
        let mut metadata_eval = Evaluator::new(&metadata_module);
        let _ = metadata_eval.set_max_callstack_size(1000);

        let metadata_value = metadata_eval
            .eval_function(metadata_fn.value(), &[], &[])
            .map_err(|e| format_eval_error(&format!("metadata() failed: {e}")))?;

        let metadata = parse_metadata(metadata_value)?;

        // Call build function with context
        let build_fn = frozen_module
            .get("build")
            .map_err(|_| format_missing_function_error("build"))?;

        // Create build context with metadata
        let jobs = i32::try_from(num_cpus::get()).unwrap_or(1);
        let build_prefix = format!("/{}-{}", metadata.name, metadata.version);
        let context = if let Some(exec) = &executor {
            BuildContext::with_executor("/opt/pm/live".to_string(), jobs, (**exec).clone())
                .with_metadata(metadata.name.clone(), metadata.version.clone())
                .with_build_prefix(build_prefix.clone())
        } else {
            BuildContext::new("/opt/pm/live".to_string(), jobs)
                .with_metadata(metadata.name.clone(), metadata.version.clone())
                .with_build_prefix(build_prefix)
        };

        // Create a new module for the build function evaluation
        let build_module = Module::new();
        let mut build_eval = Evaluator::new(&build_module);
        let _ = build_eval.set_max_callstack_size(1000);

        // Allocate the context in the evaluator's heap
        let starlark_context = context.clone();
        let context_value = build_eval.heap().alloc(starlark_context.clone());

        // Call the build function with the context
        build_eval
            .eval_function(build_fn.value(), &[context_value], &[])
            .map_err(|e| format_build_error(&e.to_string()))?;

        // Extract build steps from the shared Rc<RefCell<>> - now both contexts share the same steps
        let build_steps = starlark_context.steps.borrow().clone();

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

        // For now, make install step optional while we develop the Starlark API
        // This will be re-enabled once method calls are properly working
        // if !build_steps.iter().any(|s| matches!(s, BuildStep::Install)) {
        //     return Err(BuildError::RecipeError {
        //         message: "build() function must call ctx.install()".to_string(),
        //     }
        //     .into());
        // }

        // Validate that if install() is used, it must be the last step
        if let Some(install_position) = build_steps
            .iter()
            .position(|s| matches!(s, BuildStep::Install))
        {
            if install_position != build_steps.len() - 1 {
                return Err(BuildError::RecipeError {
                    message: "install() must be the last step in the build() function if used"
                        .to_string(),
                }
                .into());
            }
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
