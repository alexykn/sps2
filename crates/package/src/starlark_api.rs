//! Starlark API exposed to recipes - Improved with Better Argument Processing
//!
//! NOTE: The current implementation has limitations due to the Arguments type in
//! Starlark 0.13 not providing methods to iterate over or extract multiple positional
//! arguments when used in a custom `StarlarkValue`'s `invoke()` method.
//!
//! The Arguments type only provides:
//! - `positional1()` - for extracting exactly 1 positional argument
//! - `len()` - for getting the total count
//! - `no_positional_args()` - to ensure no positional args
//! - `no_named_args()` - to ensure no named args
//!
//! For proper argument handling, consider refactoring to use the `#[starlark_module]`
//! macro with proper function definitions instead of custom `StarlarkValue` implementations.

#![allow(clippy::needless_lifetimes)]

use crate::error_helpers::format_metadata_error;
use crate::recipe::{BuildStep, RecipeMetadata};
use allocative::Allocative;
use sps2_errors::Error;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Arguments;
use starlark::values::{
    AllocValue, Heap, ProvidesStaticType, StarlarkValue, Trace, UnpackValue, Value,
};
use starlark_derive::{starlark_value, NoSerialize};
use std::cell::RefCell;
use std::fmt::{self, Display};
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Trait for actual build operations that can be implemented by the builder crate
#[async_trait::async_trait]
pub trait BuildExecutor: Send + Sync + std::fmt::Debug {
    async fn fetch(&mut self, url: &str, hash: &str) -> Result<PathBuf, Error>;
    async fn make(&mut self, args: &[String]) -> Result<(), Error>;
    async fn install(&mut self) -> Result<(), Error>;
    async fn configure(&mut self, args: &[String]) -> Result<(), Error>;
    async fn autotools(&mut self, args: &[String]) -> Result<(), Error>;
    async fn cmake(&mut self, args: &[String]) -> Result<(), Error>;
    async fn meson(&mut self, args: &[String]) -> Result<(), Error>;
    async fn cargo(&mut self, args: &[String]) -> Result<(), Error>;
    async fn apply_patch(&mut self, patch_path: &Path) -> Result<(), Error>;
}

/// Build method function that can be called from Starlark
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuildMethodFunction {
    context: BuildContext,
    method_name: String,
    #[allocative(skip)]
    executor: Option<Arc<tokio::sync::Mutex<dyn BuildExecutor>>>,
}

impl Display for BuildMethodFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<build_method '{}'>", self.method_name)
    }
}

unsafe impl<'v> Trace<'v> for BuildMethodFunction {
    fn trace(&mut self, _tracer: &starlark::values::Tracer<'v>) {
        // No Value<'v> types to trace
    }
}

impl BuildMethodFunction {
    /// Handle zero-argument methods
    fn handle_no_args_method(
        &self,
        args: &Arguments<'_, '_>,
        step: BuildStep,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 0 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Method requires no arguments, got {}",
                len
            )));
        }
        args.no_named_args()?;

        self.context.steps.borrow_mut().push(step);
        Ok(())
    }

    /// Handle variable-argument methods (placeholder implementation)
    fn handle_variable_args_method(
        &self,
        args: &Arguments<'_, '_>,
        method_name: &str,
    ) -> starlark::Result<()> {
        args.no_named_args()?;

        let empty_args = Vec::new();
        let step = match method_name {
            "make" => BuildStep::Make { args: empty_args },
            "configure" => BuildStep::Configure { args: empty_args },
            "autotools" => BuildStep::Autotools { args: empty_args },
            "cmake" => BuildStep::Cmake { args: empty_args },
            "meson" => BuildStep::Meson { args: empty_args },
            "cargo" => BuildStep::Cargo { args: empty_args },
            _ => {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Unknown variable args method: {}",
                    method_name
                )))
            }
        };

        self.context.steps.borrow_mut().push(step);
        Ok(())
    }

    fn handle_fetch_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "fetch() requires exactly 1 argument: url, got {}",
                len
            )));
        }
        args.no_named_args()?;

        let url = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| starlark::Error::new_other(anyhow::anyhow!("URL must be a string")))?
            .to_string();

        let blake3 = "<blake3-placeholder>".to_string();
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::Fetch { url, blake3 });
        Ok(())
    }

    fn handle_apply_patch_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "apply_patch() requires exactly 1 argument: patch path, got {}",
                len
            )));
        }
        args.no_named_args()?;

        let patch_path = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Patch path must be a string"))
            })?
            .to_string();

        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::ApplyPatch { path: patch_path });
        Ok(())
    }

    fn handle_command_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        args.no_named_args()?;

        let len = args.len()?;
        if len < 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "command() requires at least one argument (the program to run)"
            )));
        }

        // Handle different argument patterns
        match len {
            1 => {
                // command("program") or command("program args")
                let full_command = args
                    .positional1(eval.heap())?
                    .unpack_str()
                    .ok_or_else(|| {
                        starlark::Error::new_other(anyhow::anyhow!("Command must be a string"))
                    })?
                    .to_string();

                // Split the command into program and arguments
                let parts: Vec<&str> = full_command.split_whitespace().collect();
                if parts.is_empty() {
                    return Err(starlark::Error::new_other(anyhow::anyhow!(
                        "Command cannot be empty"
                    )));
                }
                let program = parts[0].to_string();
                let cmd_args: Vec<String> = parts[1..].iter().map(ToString::to_string).collect();

                self.context.steps.borrow_mut().push(BuildStep::Command {
                    program,
                    args: cmd_args,
                });
                Ok(())
            }
            2 => {
                // command("program", "arg") - program name and single argument
                // Since we can't easily extract multiple positional args, we'll use a different approach
                // We'll try to extract all arguments as a tuple/list
                Err(starlark::Error::new_other(anyhow::anyhow!(
                    "command() with 2 arguments is not yet fully implemented. Please use command(\"program arg\") format for now."
                )))
            }
            _ => {
                Err(starlark::Error::new_other(anyhow::anyhow!(
                    "command() currently supports 1 argument only, got {}. Use command(\"program arg\") format.",
                    len
                )))
            }
        }
    }

    fn handle_detect_build_system_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        args.no_positional_args(eval.heap())?;
        args.no_named_args()?;
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::DetectBuildSystem);
        Ok(())
    }

    fn handle_set_build_system_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "set_build_system() requires exactly 1 argument: build system name, got {}",
                len
            )));
        }
        args.no_named_args()?;

        let name = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Build system name must be a string"))
            })?
            .to_string();

        self.context
            .detected_build_system
            .replace(Some(name.clone()));
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::SetBuildSystem { name });
        Ok(())
    }

    fn handle_enable_feature_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "enable_feature() requires exactly 1 argument: feature name, got {}",
                len
            )));
        }
        args.no_named_args()?;

        let name = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Feature name must be a string"))
            })?
            .to_string();

        self.context
            .features
            .borrow_mut()
            .insert(name.clone(), true);
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::EnableFeature { name });
        Ok(())
    }

    fn handle_disable_feature_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "disable_feature() requires exactly 1 argument: feature name, got {}",
                len
            )));
        }
        args.no_named_args()?;

        let name = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Feature name must be a string"))
            })?
            .to_string();

        self.context
            .features
            .borrow_mut()
            .insert(name.clone(), false);
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::DisableFeature { name });
        Ok(())
    }

    fn handle_with_features_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        // For now, simplified implementation - would need proper list parsing
        args.no_named_args()?;
        let len = args.len()?;
        if len < 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "with_features() requires at least 1 argument"
            )));
        }

        // Placeholder implementation
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::WithFeatures {
                features: vec![],
                steps: vec![],
            });
        Ok(())
    }

    fn handle_try_recover_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        args.no_named_args()?;
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "try_recover() requires exactly 1 argument: recovery strategy, got {}",
                len
            )));
        }

        let strategy = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Recovery strategy must be a string"))
            })?
            .to_string();

        self.context.steps.borrow_mut().push(BuildStep::TryRecover {
            steps: vec![],
            recovery_strategy: strategy,
        });
        Ok(())
    }

    fn handle_on_error_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "on_error() requires exactly 1 argument: handler name, got {}",
                len
            )));
        }
        args.no_named_args()?;

        let handler = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Handler name must be a string"))
            })?
            .to_string();

        self.context
            .error_handlers
            .borrow_mut()
            .push(handler.clone());
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::OnError { handler });
        Ok(())
    }

    fn handle_checkpoint_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "checkpoint() requires exactly 1 argument: checkpoint name, got {}",
                len
            )));
        }
        args.no_named_args()?;

        let name = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Checkpoint name must be a string"))
            })?
            .to_string();

        self.context.checkpoints.borrow_mut().push(name.clone());
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::Checkpoint { name });
        Ok(())
    }

    fn handle_set_target_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "set_target() requires exactly 1 argument: target triple, got {}",
                len
            )));
        }
        args.no_named_args()?;

        let triple = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Target triple must be a string"))
            })?
            .to_string();

        self.context.target_triple.replace(Some(triple.clone()));
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::SetTarget { triple });
        Ok(())
    }

    fn handle_set_toolchain_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        args.no_named_args()?;
        let len = args.len()?;
        if len != 2 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "set_toolchain() requires exactly 2 arguments: name and path, got {}",
                len
            )));
        }

        // Due to Starlark limitations, we can only get the first argument easily
        // For now, simplified implementation
        let name = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Toolchain name must be a string"))
            })?
            .to_string();

        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::SetToolchain {
                name,
                path: "/placeholder/path".to_string(),
            });
        Ok(())
    }

    fn handle_set_parallelism_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let len = args.len()?;
        if len != 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "set_parallelism() requires exactly 1 argument: number of jobs, got {}",
                len
            )));
        }
        args.no_named_args()?;

        let jobs_value = args.positional1(eval.heap())?;
        let jobs = jobs_value.unpack_i32().ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!("Jobs must be an integer"))
        })?;

        if jobs <= 0 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "Jobs must be a positive integer"
            )));
        }

        let jobs_usize = jobs.try_into().unwrap_or(1);
        self.context.parallelism.replace(jobs_usize);
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::SetParallelism { jobs: jobs_usize });
        Ok(())
    }

    fn handle_parallel_steps_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        // For now, simplified implementation - would need proper list parsing
        args.no_named_args()?;

        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::ParallelSteps { steps: vec![] });
        Ok(())
    }

    fn handle_set_resource_hints_invoke<'v>(
        &self,
        args: &Arguments<'v, '_>,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        args.no_named_args()?;

        // For now, simplified implementation
        self.context.resource_hints.replace((None, None));
        self.context
            .steps
            .borrow_mut()
            .push(BuildStep::SetResourceHints {
                cpu: None,
                memory_mb: None,
            });
        Ok(())
    }
}

#[starlark_value(type = "BuildMethodFunction")]
impl<'v> StarlarkValue<'v> for BuildMethodFunction {
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        match self.method_name.as_str() {
            "fetch" => {
                self.handle_fetch_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "make" | "configure" | "autotools" | "cmake" | "meson" | "cargo" => {
                self.handle_variable_args_method(args, &self.method_name)?;
                Ok(Value::new_none())
            }
            "install" => {
                self.handle_no_args_method(args, BuildStep::Install)?;
                Ok(Value::new_none())
            }
            "apply_patch" => {
                self.handle_apply_patch_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "command" => {
                self.handle_command_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "detect_build_system" => {
                self.handle_detect_build_system_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "set_build_system" => {
                self.handle_set_build_system_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "enable_feature" => {
                self.handle_enable_feature_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "disable_feature" => {
                self.handle_disable_feature_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "with_features" => {
                self.handle_with_features_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "try_recover" => {
                self.handle_try_recover_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "on_error" => {
                self.handle_on_error_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "checkpoint" => {
                self.handle_checkpoint_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "set_target" => {
                self.handle_set_target_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "set_toolchain" => {
                self.handle_set_toolchain_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "set_parallelism" => {
                self.handle_set_parallelism_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "parallel_steps" => {
                self.handle_parallel_steps_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            "set_resource_hints" => {
                self.handle_set_resource_hints_invoke(args, eval)?;
                Ok(Value::new_none())
            }
            _ => Err(starlark::Error::new_other(anyhow::anyhow!(
                "unknown method: {}",
                self.method_name
            ))),
        }
    }
}

impl<'v> AllocValue<'v> for BuildMethodFunction {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for BuildMethodFunction {
    type Error = starlark::Error;

    fn unpack_value(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.request_value::<&BuildMethodFunction>().cloned())
    }

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.request_value::<&BuildMethodFunction>().cloned())
    }
}

/// Build context exposed to Starlark recipes
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuildContext {
    #[allocative(skip)]
    pub steps: Rc<RefCell<Vec<BuildStep>>>,
    pub prefix: String,
    pub jobs: i32,
    #[allocative(skip)]
    pub network_allowed: RefCell<bool>,
    // Metadata that can be accessed in build()
    pub name: String,
    pub version: String,
    // Build executor integration
    #[allocative(skip)]
    executor: Option<Arc<tokio::sync::Mutex<dyn BuildExecutor>>>,
    // Build system detection
    #[allocative(skip)]
    pub detected_build_system: RefCell<Option<String>>,
    // Feature flags
    #[allocative(skip)]
    pub features: Rc<RefCell<std::collections::HashMap<String, bool>>>,
    // Error recovery
    #[allocative(skip)]
    pub error_handlers: Rc<RefCell<Vec<String>>>,
    #[allocative(skip)]
    pub checkpoints: Rc<RefCell<Vec<String>>>,
    // Cross-compilation
    #[allocative(skip)]
    pub target_triple: RefCell<Option<String>>,
    #[allocative(skip)]
    pub toolchain: RefCell<std::collections::HashMap<String, String>>,
    // Parallel configuration
    pub parallelism: RefCell<usize>,
    #[allocative(skip)]
    pub resource_hints: RefCell<(Option<usize>, Option<usize>)>, // (cpu, memory_mb)
}

impl BuildContext {
    #[must_use]
    pub fn new(prefix: String, jobs: i32) -> Self {
        Self {
            steps: Rc::new(RefCell::new(Vec::new())),
            prefix,
            jobs,
            network_allowed: RefCell::new(false),
            name: String::new(),
            version: String::new(),
            executor: None,
            detected_build_system: RefCell::new(None),
            features: Rc::new(RefCell::new(std::collections::HashMap::new())),
            error_handlers: Rc::new(RefCell::new(Vec::new())),
            checkpoints: Rc::new(RefCell::new(Vec::new())),
            target_triple: RefCell::new(None),
            toolchain: RefCell::new(std::collections::HashMap::new()),
            parallelism: RefCell::new(jobs.try_into().unwrap_or(1)),
            resource_hints: RefCell::new((None, None)),
        }
    }

    /// Create a new build context with executor integration
    #[must_use]
    pub fn with_executor(
        prefix: String,
        jobs: i32,
        executor: Arc<tokio::sync::Mutex<dyn BuildExecutor>>,
    ) -> Self {
        Self {
            steps: Rc::new(RefCell::new(Vec::new())),
            prefix,
            jobs,
            network_allowed: RefCell::new(false),
            name: String::new(),
            version: String::new(),
            executor: Some(executor),
            detected_build_system: RefCell::new(None),
            features: Rc::new(RefCell::new(std::collections::HashMap::new())),
            error_handlers: Rc::new(RefCell::new(Vec::new())),
            checkpoints: Rc::new(RefCell::new(Vec::new())),
            target_triple: RefCell::new(None),
            toolchain: RefCell::new(std::collections::HashMap::new()),
            parallelism: RefCell::new(jobs.try_into().unwrap_or(1)),
            resource_hints: RefCell::new((None, None)),
        }
    }

    #[must_use]
    pub fn with_metadata(mut self, name: String, version: String) -> Self {
        self.name = name;
        self.version = version;
        self
    }

    /// Fetch a source archive from the given URL with BLAKE3 verification
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation,
    /// but will return errors in the full builder implementation for network failures,
    /// invalid URLs, or BLAKE3 verification failures.
    pub fn fetch(&self, url: &str, blake3: &str) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Fetch {
            url: url.to_string(),
            blake3: blake3.to_string(),
        });
        Ok(())
    }

    /// Run make with the specified arguments
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation,
    /// but will return errors in the full builder implementation for make failures
    /// or invalid arguments.
    pub fn make(&self, args: Vec<String>) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Make { args });
        Ok(())
    }

    /// Install the built package to the target directory
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation,
    /// but will return errors in the full builder implementation for installation
    /// failures or filesystem errors.
    pub fn install(&self) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Install);
        Ok(())
    }

    /// Run configure script with the specified arguments
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation,
    /// but will return errors in the full builder implementation for configure script
    /// failures or invalid arguments.
    pub fn configure(&self, args: Vec<String>) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Configure { args });
        Ok(())
    }

    /// Run autotools (configure && make && make install) with the specified arguments
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn autotools(&self, args: Vec<String>) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Autotools { args });
        Ok(())
    }

    /// Run cmake build with the specified arguments
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn cmake(&self, args: Vec<String>) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Cmake { args });
        Ok(())
    }

    /// Run meson build with the specified arguments
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn meson(&self, args: Vec<String>) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Meson { args });
        Ok(())
    }

    /// Run cargo build with the specified arguments
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn cargo(&self, args: Vec<String>) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Cargo { args });
        Ok(())
    }

    /// Apply a patch file
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn apply_patch(&self, path: &str) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::ApplyPatch {
            path: path.to_string(),
        });
        Ok(())
    }

    /// Run an arbitrary command with arguments
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn command(&self, program: &str, args: Vec<String>) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Command {
            program: program.to_string(),
            args,
        });
        Ok(())
    }

    /// Detect the build system for the current source directory
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn detect_build_system(&self) -> anyhow::Result<String> {
        self.steps.borrow_mut().push(BuildStep::DetectBuildSystem);
        // Return a placeholder for now - actual detection happens in builder
        Ok("autodetect".to_string())
    }

    /// Set the build system to use
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn set_build_system(&self, name: &str) -> anyhow::Result<()> {
        self.detected_build_system.replace(Some(name.to_string()));
        self.steps.borrow_mut().push(BuildStep::SetBuildSystem {
            name: name.to_string(),
        });
        Ok(())
    }

    /// Enable a feature flag
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn enable_feature(&self, name: &str) -> anyhow::Result<()> {
        self.features.borrow_mut().insert(name.to_string(), true);
        self.steps.borrow_mut().push(BuildStep::EnableFeature {
            name: name.to_string(),
        });
        Ok(())
    }

    /// Disable a feature flag
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn disable_feature(&self, name: &str) -> anyhow::Result<()> {
        self.features.borrow_mut().insert(name.to_string(), false);
        self.steps.borrow_mut().push(BuildStep::DisableFeature {
            name: name.to_string(),
        });
        Ok(())
    }

    /// Execute steps conditionally based on features
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn with_features(
        &self,
        features: Vec<String>,
        steps: Vec<BuildStep>,
    ) -> anyhow::Result<()> {
        self.steps
            .borrow_mut()
            .push(BuildStep::WithFeatures { features, steps });
        Ok(())
    }

    /// Try to recover from errors with specified strategy
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn try_recover(
        &self,
        steps: Vec<BuildStep>,
        recovery_strategy: &str,
    ) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::TryRecover {
            steps,
            recovery_strategy: recovery_strategy.to_string(),
        });
        Ok(())
    }

    /// Register an error handler
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn on_error(&self, handler: &str) -> anyhow::Result<()> {
        self.error_handlers.borrow_mut().push(handler.to_string());
        self.steps.borrow_mut().push(BuildStep::OnError {
            handler: handler.to_string(),
        });
        Ok(())
    }

    /// Create a checkpoint for recovery
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn checkpoint(&self, name: &str) -> anyhow::Result<()> {
        self.checkpoints.borrow_mut().push(name.to_string());
        self.steps.borrow_mut().push(BuildStep::Checkpoint {
            name: name.to_string(),
        });
        Ok(())
    }

    /// Set the target triple for cross-compilation
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn set_target(&self, triple: &str) -> anyhow::Result<()> {
        self.target_triple.replace(Some(triple.to_string()));
        self.steps.borrow_mut().push(BuildStep::SetTarget {
            triple: triple.to_string(),
        });
        Ok(())
    }

    /// Set a toolchain component for cross-compilation
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn set_toolchain(&self, name: &str, path: &str) -> anyhow::Result<()> {
        self.toolchain
            .borrow_mut()
            .insert(name.to_string(), path.to_string());
        self.steps.borrow_mut().push(BuildStep::SetToolchain {
            name: name.to_string(),
            path: path.to_string(),
        });
        Ok(())
    }

    /// Set the parallelism level for builds
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn set_parallelism(&self, jobs: usize) -> anyhow::Result<()> {
        self.parallelism.replace(jobs);
        self.steps
            .borrow_mut()
            .push(BuildStep::SetParallelism { jobs });
        Ok(())
    }

    /// Execute steps in parallel
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn parallel_steps(&self, steps: Vec<BuildStep>) -> anyhow::Result<()> {
        self.steps
            .borrow_mut()
            .push(BuildStep::ParallelSteps { steps });
        Ok(())
    }

    /// Set resource hints for the build
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation.
    pub fn set_resource_hints(
        &self,
        cpu: Option<usize>,
        memory_mb: Option<usize>,
    ) -> anyhow::Result<()> {
        self.resource_hints.replace((cpu, memory_mb));
        self.steps
            .borrow_mut()
            .push(BuildStep::SetResourceHints { cpu, memory_mb });
        Ok(())
    }
}

impl Display for BuildContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BuildContext(prefix={}, jobs={}, name={}, version={})",
            self.prefix, self.jobs, self.name, self.version
        )
    }
}

unsafe impl<'v> Trace<'v> for BuildContext {
    fn trace(&mut self, _tracer: &starlark::values::Tracer<'v>) {
        // No Value<'v> types to trace in BuildContext
    }
}

impl BuildContext {
    /// Helper to create build system method functions
    fn create_build_system_method<'v>(&self, method_name: &str, heap: &'v Heap) -> Value<'v> {
        heap.alloc(BuildMethodFunction {
            context: self.clone(),
            method_name: method_name.to_string(),
            executor: self.executor.clone(),
        })
    }

    /// Helper to create build operation method functions
    fn create_build_operation_method<'v>(&self, method_name: &str, heap: &'v Heap) -> Value<'v> {
        heap.alloc(BuildMethodFunction {
            context: self.clone(),
            method_name: method_name.to_string(),
            executor: self.executor.clone(),
        })
    }

    /// Helper to create feature management method functions
    fn create_feature_method<'v>(&self, method_name: &str, heap: &'v Heap) -> Value<'v> {
        heap.alloc(BuildMethodFunction {
            context: self.clone(),
            method_name: method_name.to_string(),
            executor: self.executor.clone(),
        })
    }

    /// Helper to create recovery and error handling method functions
    fn create_recovery_method<'v>(&self, method_name: &str, heap: &'v Heap) -> Value<'v> {
        heap.alloc(BuildMethodFunction {
            context: self.clone(),
            method_name: method_name.to_string(),
            executor: self.executor.clone(),
        })
    }

    /// Helper to create configuration and optimization method functions
    fn create_config_method<'v>(&self, method_name: &str, heap: &'v Heap) -> Value<'v> {
        heap.alloc(BuildMethodFunction {
            context: self.clone(),
            method_name: method_name.to_string(),
            executor: self.executor.clone(),
        })
    }
}

#[starlark_value(type = "BuildContext")]
impl<'v> StarlarkValue<'v> for BuildContext {
    fn has_attr(&self, attribute: &str, _heap: &'v Heap) -> bool {
        matches!(
            attribute,
            "PREFIX"
                | "JOBS"
                | "NAME"
                | "VERSION"
                | "fetch"
                | "make"
                | "install"
                | "configure"
                | "autotools"
                | "cmake"
                | "meson"
                | "cargo"
                | "apply_patch"
                | "command"
                | "detect_build_system"
                | "set_build_system"
                | "enable_feature"
                | "disable_feature"
                | "with_features"
                | "try_recover"
                | "on_error"
                | "checkpoint"
                | "set_target"
                | "set_toolchain"
                | "set_parallelism"
                | "parallel_steps"
                | "set_resource_hints"
        )
    }

    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<Value<'v>> {
        match attribute {
            "PREFIX" => Some(heap.alloc(&self.prefix)),
            "JOBS" => Some(heap.alloc(self.jobs)),
            "NAME" => Some(heap.alloc(&self.name)),
            "VERSION" => Some(heap.alloc(&self.version)),
            // Basic build system methods
            "fetch" | "make" | "install" | "configure" | "autotools" | "cmake" | "meson"
            | "cargo" => Some(self.create_build_system_method(attribute, heap)),
            // Build operation methods
            "apply_patch" | "command" | "detect_build_system" | "set_build_system" => {
                Some(self.create_build_operation_method(attribute, heap))
            }
            // Feature management methods
            "enable_feature" | "disable_feature" | "with_features" => {
                Some(self.create_feature_method(attribute, heap))
            }
            // Error handling and recovery methods
            "try_recover" | "on_error" | "checkpoint" => {
                Some(self.create_recovery_method(attribute, heap))
            }
            // Configuration and optimization methods
            "set_target" | "set_toolchain" | "set_parallelism" | "parallel_steps"
            | "set_resource_hints" => Some(self.create_config_method(attribute, heap)),
            _ => None,
        }
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        _args: &starlark::eval::Arguments<'v, '_>,
        _eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // For now, let's simplify this to just return None
        // The actual method dispatch will be implemented when we have a proper Starlark integration
        // This is a placeholder to allow compilation
        Ok(Value::new_none())
    }
}

impl<'v> AllocValue<'v> for BuildContext {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for BuildContext {
    type Error = starlark::Error;

    fn unpack_value(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.request_value::<&BuildContext>().cloned())
    }

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<Self>, Self::Error> {
        Ok(value.request_value::<&BuildContext>().cloned())
    }
}

/// Simple build API for now - we'll enhance this later
pub fn build_api(_builder: &mut GlobalsBuilder) {
    // For now, just create a basic globals environment
    // We'll add functions later when we have the basics working
}

/// Helper function to extract a list of strings from a Starlark dict
fn get_string_list(
    dict: &starlark::values::dict::DictRef,
    key: &str,
) -> Result<Vec<String>, Error> {
    use starlark::values::list::ListRef;

    match dict.get_str(key) {
        Some(val) => {
            // Try to get as a list
            if let Some(list) = ListRef::from_value(val) {
                let mut result = Vec::new();
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        result.push(s.to_string());
                    } else {
                        return Err(format_metadata_error(
                            key,
                            &format!(
                                "dependency list '{}' must contain only strings, got: {}",
                                key,
                                item.get_type()
                            ),
                        )
                        .into());
                    }
                }
                Ok(result)
            } else {
                Err(format_metadata_error(
                    key,
                    &format!(
                        "metadata field '{}' must be a list, got: {}",
                        key,
                        val.get_type()
                    ),
                )
                .into())
            }
        }
        None => Ok(Vec::new()), // Field is optional, return empty list
    }
}

/// Parse metadata from Starlark value (dict)
pub fn parse_metadata(value: Value) -> Result<RecipeMetadata, Error> {
    use starlark::values::dict::DictRef;

    let mut metadata = RecipeMetadata::default();

    // Try to get the dict ref from the value
    let dict = DictRef::from_value(value).ok_or_else(|| {
        format_metadata_error(
            "type",
            &format!(
                "metadata() must return a dictionary, got: {}",
                value.get_type()
            ),
        )
    })?;

    // Helper function to get string value from dict with type validation
    let get_string_value = |key: &str| -> Result<Option<String>, Error> {
        match dict.get_str(key) {
            Some(val) => {
                if let Some(s) = val.unpack_str() {
                    Ok(Some(s.to_string()))
                } else {
                    Err(format_metadata_error(
                        key,
                        &format!(
                            "metadata field '{}' must be a string, got: {}",
                            key,
                            val.get_type()
                        ),
                    )
                    .into())
                }
            }
            None => Ok(None),
        }
    };

    // Extract name (required)
    metadata.name = get_string_value("name")?
        .ok_or_else(|| format_metadata_error("name", "metadata must include 'name' field"))?;

    if metadata.name.trim().is_empty() {
        return Err(format_metadata_error("name", "metadata 'name' cannot be empty").into());
    }

    // Extract version (required)
    metadata.version = get_string_value("version")?
        .ok_or_else(|| format_metadata_error("version", "metadata must include 'version' field"))?;

    if metadata.version.trim().is_empty() {
        return Err(format_metadata_error("version", "metadata 'version' cannot be empty").into());
    }

    // Extract optional fields
    metadata.description = get_string_value("description")?;
    metadata.homepage = get_string_value("homepage")?;
    metadata.license = get_string_value("license")?;

    // Extract dependency arrays
    metadata.runtime_deps = get_string_list(&dict, "depends")?;
    metadata.build_deps = get_string_list(&dict, "build_depends")?;

    Ok(metadata)
}
