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

        // For simplicity, we'll only support the form: command("program", ["arg1", "arg2"])
        let len = args.len()?;
        if len < 1 {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "command() requires at least one argument (the program to run)"
            )));
        }

        // Get program name
        let program = args
            .positional1(eval.heap())?
            .unpack_str()
            .ok_or_else(|| {
                starlark::Error::new_other(anyhow::anyhow!("Program name must be a string"))
            })?
            .to_string();

        // For now, we'll just use empty args. In the future we can parse a list from the second argument
        let cmd_args = Vec::new();

        self.context.steps.borrow_mut().push(BuildStep::Command {
            program,
            args: cmd_args,
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
        )
    }

    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<Value<'v>> {
        match attribute {
            "PREFIX" => Some(heap.alloc(&self.prefix)),
            "JOBS" => Some(heap.alloc(self.jobs)),
            "NAME" => Some(heap.alloc(&self.name)),
            "VERSION" => Some(heap.alloc(&self.version)),
            "fetch" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "fetch".to_string(),
                executor: self.executor.clone(),
            })),
            "make" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "make".to_string(),
                executor: self.executor.clone(),
            })),
            "install" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "install".to_string(),
                executor: self.executor.clone(),
            })),
            "configure" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "configure".to_string(),
                executor: self.executor.clone(),
            })),
            "autotools" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "autotools".to_string(),
                executor: self.executor.clone(),
            })),
            "cmake" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "cmake".to_string(),
                executor: self.executor.clone(),
            })),
            "meson" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "meson".to_string(),
                executor: self.executor.clone(),
            })),
            "cargo" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "cargo".to_string(),
                executor: self.executor.clone(),
            })),
            "apply_patch" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "apply_patch".to_string(),
                executor: self.executor.clone(),
            })),
            "command" => Some(heap.alloc(BuildMethodFunction {
                context: self.clone(),
                method_name: "command".to_string(),
                executor: self.executor.clone(),
            })),
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
