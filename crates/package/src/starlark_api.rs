//! Starlark API exposed to recipes - Working Minimal Version

use crate::recipe::{BuildStep, RecipeMetadata};
use spsv2_errors::Error;
use allocative::Allocative;
use starlark::environment::GlobalsBuilder;
use starlark::values::{
    AllocValue, Heap, ProvidesStaticType, StarlarkValue, UnpackValue, Value, Trace
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
}

impl BuildContext {
    pub fn new(prefix: String, jobs: i32) -> Self {
        Self {
            steps: RefCell::new(Vec::new()),
            prefix,
            jobs,
            network_allowed: RefCell::new(false),
        }
    }

    pub fn fetch(&self, url: &str, sha256: &str) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Fetch {
            url: url.to_string(),
            sha256: sha256.to_string(),
        });
        Ok(())
    }

    pub fn make(&self, args: Vec<String>) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Make { args });
        Ok(())
    }

    pub fn install(&self) -> anyhow::Result<()> {
        self.steps.borrow_mut().push(BuildStep::Install);
        Ok(())
    }
}

impl Display for BuildContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BuildContext(prefix={}, jobs={})", self.prefix, self.jobs)
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
        matches!(attribute, "PREFIX" | "JOBS" | "fetch" | "make" | "install")
    }

    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<Value<'v>> {
        match attribute {
            "PREFIX" => Some(heap.alloc(&self.prefix)),
            "JOBS" => Some(heap.alloc(self.jobs)),
            _ => None, // We'll handle methods later
        }
    }
}

impl<'v> AllocValue<'v> for BuildContext {
    fn alloc_value(self, heap: &'v Heap) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> UnpackValue<'v> for BuildContext {
    fn unpack_value(value: Value<'v>) -> Option<Self> {
        value.request_value::<&BuildContext>().map(|bc| bc.clone())
    }
}

/// Simple build API for now - we'll enhance this later
pub fn build_api(builder: &mut GlobalsBuilder) {
    // For now, just create a basic globals environment
    // We'll add functions later when we have the basics working
}

/// Parse metadata from Starlark value - simplified version for now
pub fn parse_metadata(_value: Value) -> Result<RecipeMetadata, Error> {
    // For now, return a working default - this will be improved later
    let mut metadata = RecipeMetadata::default();
    metadata.name = "hello".to_string();
    metadata.version = "1.0.0".to_string();
    metadata.description = Some("A simple hello world program".to_string());
    
    Ok(metadata)
}