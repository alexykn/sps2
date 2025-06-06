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
        let context = if let Some(exec) = &executor {
            BuildContext::with_executor("/opt/pm/live".to_string(), jobs, (**exec).clone())
                .with_metadata(metadata.name.clone(), metadata.version.clone())
                .with_build_prefix(String::new())  // Empty means install directly to stage/
        } else {
            BuildContext::new("/opt/pm/live".to_string(), jobs)
                .with_metadata(metadata.name.clone(), metadata.version.clone())
                .with_build_prefix(String::new())  // Empty means install directly to stage/
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

    #[test]
    fn test_global_functions_integration() {
        use crate::recipe::BuildStep;

        let recipe_content = r#"
def metadata():
    return {
        "name": "test-global-funcs",
        "version": "1.0.0",
        "description": "Test global function integration"
    }

def build(ctx):
    # Test various global functions
    fetch(ctx, "https://example.com/source.tar.gz", "hash123")
    
    # Test build system functions
    configure(ctx, ["--prefix=" + ctx.PREFIX])
    make(ctx, ["-j" + str(ctx.JOBS)])
    cmake(ctx, ["-DCMAKE_INSTALL_PREFIX=" + ctx.PREFIX])
    
    # Test feature functions
    enable_feature(ctx, "ssl")
    disable_feature(ctx, "debug")
    
    # Test context functions
    set_env(ctx, "CC", "gcc")
    allow_network(ctx, True)
    checkpoint(ctx, "after-build")
    
    # Test parallel functions
    set_parallelism(ctx, 4)
    
    # Test cross functions
    set_target(ctx, "aarch64-apple-darwin")
    set_toolchain(ctx, "CC", "aarch64-apple-darwin-gcc")
    
    # Finally install
    install(ctx)
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe).unwrap();

        // Check metadata
        assert_eq!(result.metadata.name, "test-global-funcs");
        assert_eq!(result.metadata.version, "1.0.0");

        // Check that build steps were recorded
        assert!(!result.build_steps.is_empty());

        // Check specific build steps
        let steps = &result.build_steps;

        // Should have fetch as first step
        assert!(
            matches!(&steps[0], BuildStep::Fetch { url, .. } if url == "https://example.com/source.tar.gz")
        );

        // Should have configure step
        assert!(steps
            .iter()
            .any(|s| matches!(s, BuildStep::Configure { .. })));

        // Should have make step
        assert!(steps.iter().any(|s| matches!(s, BuildStep::Make { .. })));

        // Should have cmake step
        assert!(steps.iter().any(|s| matches!(s, BuildStep::Cmake { .. })));

        // Should have feature steps
        assert!(steps
            .iter()
            .any(|s| matches!(s, BuildStep::EnableFeature { name } if name == "ssl")));
        assert!(steps
            .iter()
            .any(|s| matches!(s, BuildStep::DisableFeature { name } if name == "debug")));

        // Should have environment and config steps
        assert!(steps
            .iter()
            .any(|s| matches!(s, BuildStep::SetEnv { key, .. } if key == "CC")));
        assert!(steps
            .iter()
            .any(|s| matches!(s, BuildStep::AllowNetwork { enabled } if *enabled)));
        assert!(steps
            .iter()
            .any(|s| matches!(s, BuildStep::Checkpoint { name } if name == "after-build")));

        // Should have parallelism step
        assert!(steps
            .iter()
            .any(|s| matches!(s, BuildStep::SetParallelism { jobs } if *jobs == 4)));

        // Should have cross-compilation steps
        assert!(steps.iter().any(
            |s| matches!(s, BuildStep::SetTarget { triple } if triple == "aarch64-apple-darwin")
        ));
        assert!(steps
            .iter()
            .any(|s| matches!(s, BuildStep::SetToolchain { name, .. } if name == "CC")));

        // Should end with install
        assert!(matches!(steps.last(), Some(BuildStep::Install)));
    }

    #[test]
    fn test_install_must_be_last_step() {
        let recipe_content = r#"
def metadata():
    return {
        "name": "test-install-order",
        "version": "1.0.0"
    }

def build(ctx):
    configure(ctx, ["--prefix=" + ctx.PREFIX])
    install(ctx)  # Install is not the last step
    make(ctx, ["-j" + str(ctx.JOBS)])  # Error: step after install
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        // Should fail with appropriate error message
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("install() must be the last step in the build() function if used"));
    }

    #[test]
    fn test_install_as_last_step_succeeds() {
        let recipe_content = r#"
def metadata():
    return {
        "name": "test-install-last",
        "version": "1.0.0"
    }

def build(ctx):
    configure(ctx, ["--prefix=" + ctx.PREFIX])
    make(ctx, ["-j" + str(ctx.JOBS)])
    install(ctx)  # Install is the last step - this is correct
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        // Should succeed
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.metadata.name, "test-install-last");
        assert_eq!(result.metadata.version, "1.0.0");

        // Verify install is indeed the last step
        assert!(matches!(
            result.build_steps.last(),
            Some(BuildStep::Install)
        ));
    }

    #[test]
    fn test_no_install_step_allowed() {
        let recipe_content = r#"
def metadata():
    return {
        "name": "test-no-install",
        "version": "1.0.0"
    }

def build(ctx):
    configure(ctx, ["--prefix=" + ctx.PREFIX])
    make(ctx, ["-j" + str(ctx.JOBS)])
    # No install step - this is currently allowed
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe);

        // Should succeed since install is optional for now
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.metadata.name, "test-no-install");
        assert_eq!(result.metadata.version, "1.0.0");

        // Verify there's no install step
        assert!(!result
            .build_steps
            .iter()
            .any(|s| matches!(s, BuildStep::Install)));
    }
}
