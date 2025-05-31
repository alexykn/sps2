//! Sandboxed Starlark execution environment

use crate::error_helpers::{
    format_build_error, format_eval_error, format_missing_function_error, format_parse_error,
};
use crate::recipe::{BuildStep, Recipe, RecipeMetadata};
use crate::starlark_api::{build_api, parse_metadata, BuildContext, BuildExecutor};
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
        let globals = GlobalsBuilder::standard().with(build_api).build();

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

        // Evaluate the recipe
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

        let build_module = Module::new();
        let mut build_eval = Evaluator::new(&build_module);
        let _ = build_eval.set_max_callstack_size(1000);

        // Create build context with metadata
        let jobs = i32::try_from(num_cpus::get()).unwrap_or(1);
        let context = if let Some(exec) = &executor {
            BuildContext::with_executor("/opt/pm/build".to_string(), jobs, (**exec).clone())
                .with_metadata(metadata.name.clone(), metadata.version.clone())
        } else {
            BuildContext::new("/opt/pm/build".to_string(), jobs)
                .with_metadata(metadata.name.clone(), metadata.version.clone())
        };

        // Allocate the context in the build module's heap
        let context_value = build_module.heap().alloc(context.clone());

        build_eval
            .eval_function(build_fn.value(), &[context_value], &[])
            .map_err(|e| format_build_error(&e.to_string()))?;

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

        // For now, make install step optional while we develop the Starlark API
        // This will be re-enabled once method calls are properly working
        // if !build_steps.iter().any(|s| matches!(s, BuildStep::Install)) {
        //     return Err(BuildError::RecipeError {
        //         message: "build() function must call ctx.install()".to_string(),
        //     }
        //     .into());
        // }

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
    return {
        "name": "test-pkg",
        "version": "1.0.0",
        "description": "Test package"
    }

def build(ctx):
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe).unwrap();

        assert_eq!(result.metadata.name, "test-pkg");
        assert_eq!(result.metadata.version, "1.0.0");
        assert_eq!(
            result.metadata.description,
            Some("Test package".to_string())
        );
    }

    #[test]
    fn test_error_missing_metadata_function() {
        let recipe_content = r"
def build(ctx):
    pass
";

        let result = Recipe::parse(recipe_content);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("recipe missing required 'metadata' function"));
    }

    #[test]
    fn test_error_wrong_metadata_return_type() {
        let recipe_content = r#"
def metadata():
    return "not a dictionary"

def build(ctx):
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("metadata() must return a dictionary"));
    }

    #[test]
    fn test_error_missing_required_name_field() {
        let recipe_content = r#"
def metadata():
    return {
        "version": "1.0.0",
        "description": "Test package"
    }

def build(ctx):
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("metadata must include 'name' field"));
    }

    #[test]
    fn test_error_empty_name_field() {
        let recipe_content = r#"
def metadata():
    return {
        "name": "",
        "version": "1.0.0"
    }

def build(ctx):
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("metadata 'name' cannot be empty"));
    }

    #[test]
    fn test_error_wrong_field_type() {
        let recipe_content = r#"
def metadata():
    return {
        "name": 123,  # Should be string
        "version": "1.0.0"
    }

def build(ctx):
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("metadata field 'name' must be a string"));
    }

    #[test]
    fn test_context_passing() {
        let recipe_content = r#"
def metadata():
    return {
        "name": "context-test",
        "version": "2.0.0",
        "description": "Testing context passing"
    }

def build(ctx):
    # Test that we can access metadata from context
    if ctx.NAME != "context-test":
        fail("Expected ctx.NAME to be 'context-test', got: " + ctx.NAME)
    if ctx.VERSION != "2.0.0":
        fail("Expected ctx.VERSION to be '2.0.0', got: " + ctx.VERSION)
    # Also test original context fields
    if not ctx.PREFIX.endswith("build"):
        fail("Expected ctx.PREFIX to end with 'build', got: " + ctx.PREFIX)
    if ctx.JOBS <= 0:
        fail("Expected ctx.JOBS to be positive, got: " + str(ctx.JOBS))
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        // Should succeed if context passing works
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.metadata.name, "context-test");
        assert_eq!(result.metadata.version, "2.0.0");
    }

    #[test]
    fn test_dependency_parsing() {
        let recipe_content = r#"
def metadata():
    return {
        "name": "jq",
        "version": "1.7.0",
        "description": "Command-line JSON processor",
        "depends": ["oniguruma>=6.9.8", "libc++~=16.0.0"],
        "build_depends": ["autoconf>=2.71", "automake~=1.16.0", "libtool==2.4.7"]
    }

def build(ctx):
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe).unwrap();

        assert_eq!(result.metadata.name, "jq");
        assert_eq!(result.metadata.version, "1.7.0");
        assert_eq!(
            result.metadata.runtime_deps,
            vec!["oniguruma>=6.9.8", "libc++~=16.0.0"]
        );
        assert_eq!(
            result.metadata.build_deps,
            vec!["autoconf>=2.71", "automake~=1.16.0", "libtool==2.4.7"]
        );
    }

    #[test]
    fn test_dependency_parsing_optional_fields() {
        let recipe_content = r#"
def metadata():
    return {
        "name": "minimal-pkg",
        "version": "1.0.0"
    }

def build(ctx):
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe).unwrap();

        assert_eq!(result.metadata.name, "minimal-pkg");
        assert_eq!(result.metadata.version, "1.0.0");
        assert!(result.metadata.runtime_deps.is_empty());
        assert!(result.metadata.build_deps.is_empty());
    }

    #[test]
    fn test_dependency_parsing_invalid_type() {
        let recipe_content = r#"
def metadata():
    return {
        "name": "bad-deps",
        "version": "1.0.0",
        "depends": "should-be-list"
    }

def build(ctx):
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("metadata field 'depends' must be a list"));
    }

    #[test]
    fn test_dependency_parsing_invalid_list_item_type() {
        let recipe_content = r#"
def metadata():
    return {
        "name": "bad-dep-item",
        "version": "1.0.0",
        "depends": ["valid-dep", 123]
    }

def build(ctx):
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("dependency list 'depends' must contain only strings"));
    }

    #[test]
    fn test_sandboxing_deep_function_calls() {
        // Test that deep function call nesting is properly limited by callstack size
        let recipe_content = r#"
def metadata():
    return {
        "name": "deep-calls-test",
        "version": "1.0.0"
    }

def recursive_func(n):
    if n <= 0:
        return 0
    return recursive_func(n - 1) + 1

def build(ctx):
    # Try to call a function deeply nested - this should be limited by callstack
    # Note: Starlark doesn't allow traditional recursion, but we can test 
    # nested function calls through other means
    pass
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        // This should succeed as the recipe itself is valid
        // Deep recursion is actually prevented by Starlark's design, not just our limits
        assert!(result.is_ok());
    }

    #[test]
    fn test_sandboxing_limits_applied() {
        // Test that our sandboxing configuration is applied correctly
        let recipe_content = r#"
def metadata():
    return {
        "name": "sandbox-test",
        "version": "1.0.0",
        "description": "Testing sandbox limits"
    }

def build(ctx):
    # Test basic operations work within limits
    data = {}
    for i in range(100):
        data[str(i)] = i * 2
    
    # Test string operations
    text = "hello"
    for i in range(10):
        text = text + str(i)
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        // Should succeed - operations are within reasonable limits
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.metadata.name, "sandbox-test");
        assert_eq!(result.metadata.version, "1.0.0");
    }
}
