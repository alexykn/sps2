//! Starlark API exposed to recipes - Working Minimal Version

#![allow(clippy::needless_lifetimes)]

use crate::error_helpers::format_metadata_error;
use crate::recipe::{BuildStep, RecipeMetadata};
use allocative::Allocative;
use spsv2_errors::Error;
use starlark::environment::GlobalsBuilder;
use starlark::values::{
    AllocValue, Heap, ProvidesStaticType, StarlarkValue, Trace, UnpackValue, Value,
};
use starlark_derive::{starlark_value, NoSerialize};
use std::cell::RefCell;
use std::fmt::{self, Display};
/// Build context exposed to Starlark recipes
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuildContext {
    #[allocative(skip)]
    pub steps: RefCell<Vec<BuildStep>>,
    pub prefix: String,
    pub jobs: i32,
    #[allocative(skip)]
    pub network_allowed: RefCell<bool>,
    // Metadata that can be accessed in build()
    pub name: String,
    pub version: String,
}

impl BuildContext {
    #[must_use]
    pub fn new(prefix: String, jobs: i32) -> Self {
        Self {
            steps: RefCell::new(Vec::new()),
            prefix,
            jobs,
            network_allowed: RefCell::new(false),
            name: String::new(),
            version: String::new(),
        }
    }

    #[must_use]
    pub fn with_metadata(mut self, name: String, version: String) -> Self {
        self.name = name;
        self.version = version;
        self
    }

    /// Fetch a source archive from the given URL with SHA256 verification
    ///
    /// # Errors
    ///
    /// This method currently does not return errors in this minimal implementation,
    /// but will return errors in the full builder implementation for network failures,
    /// invalid URLs, or SHA256 verification failures.
    pub fn fetch(&self, url: &str, sha256: &str) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Fetch {
            url: url.to_string(),
            sha256: sha256.to_string(),
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
            "PREFIX" | "JOBS" | "NAME" | "VERSION" | "fetch" | "make" | "install" | "configure"
        )
    }

    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<Value<'v>> {
        match attribute {
            "PREFIX" => Some(heap.alloc(&self.prefix)),
            "JOBS" => Some(heap.alloc(self.jobs)),
            "NAME" => Some(heap.alloc(&self.name)),
            "VERSION" => Some(heap.alloc(&self.version)),
            _ => None, // Methods will be handled via invoke
        }
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        _args: &starlark::eval::Arguments<'v, '_>,
        _eval: &mut starlark::eval::Evaluator<'v, '_>,
    ) -> starlark::Result<Value<'v>> {
        // For now, let's remove method invocation and just track that methods were called
        // We'll implement this properly when we connect to the builder crate
        Ok(Value::new_none())
    }
}

impl<'v> AllocValue<'v> for BuildContext {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for BuildContext {
    fn unpack_value(value: Value<'v>) -> Option<Self> {
        value.request_value::<&BuildContext>().cloned()
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
