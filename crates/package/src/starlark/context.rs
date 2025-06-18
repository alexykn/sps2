#![allow(clippy::needless_lifetimes)]
//! Build context implementation for Starlark recipes

use crate::recipe::BuildStep;
use allocative::Allocative;
use sps2_errors::Error;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::none::NoneType;
use starlark::values::{
    AllocValue, Heap, ProvidesStaticType, StarlarkValue, Trace, UnpackValue, Value, ValueLike,
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
    async fn fetch(&mut self, url: &str) -> Result<PathBuf, Error>;
    async fn fetch_md5(&mut self, url: &str, expected_md5: &str) -> Result<PathBuf, Error>;
    async fn fetch_sha256(&mut self, url: &str, expected_sha256: &str) -> Result<PathBuf, Error>;
    async fn fetch_blake3(&mut self, url: &str, expected_blake3: &str) -> Result<PathBuf, Error>;
    async fn git(&mut self, url: &str, ref_: &str) -> Result<PathBuf, Error>;
    async fn make(&mut self, args: &[String]) -> Result<(), Error>;
    async fn install(&mut self) -> Result<(), Error>;
    async fn configure(&mut self, args: &[String]) -> Result<(), Error>;
    async fn autotools(&mut self, args: &[String]) -> Result<(), Error>;
    async fn cmake(&mut self, args: &[String]) -> Result<(), Error>;
    async fn meson(&mut self, args: &[String]) -> Result<(), Error>;
    async fn cargo(&mut self, args: &[String]) -> Result<(), Error>;
    async fn go(&mut self, args: &[String]) -> Result<(), Error>;
    async fn python(&mut self, args: &[String]) -> Result<(), Error>;
    async fn nodejs(&mut self, args: &[String]) -> Result<(), Error>;
    async fn apply_patch(&mut self, patch_path: &Path) -> Result<(), Error>;
    async fn copy(&mut self, src_path: Option<&str>) -> Result<(), Error>;
    async fn with_defaults(&mut self) -> Result<(), Error>;
}

/// Build context exposed to Starlark recipes
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuildContext {
    #[allocative(skip)]
    pub steps: Rc<RefCell<Vec<BuildStep>>>,
    pub prefix: String,       // Final installation prefix (e.g., /opt/pm/live)
    pub build_prefix: String, // Staging directory prefix (relative to stage/)
    pub jobs: i32,
    #[allocative(skip)]
    pub network_allowed: RefCell<bool>,
    // Metadata that can be accessed in build()
    pub name: String,
    pub version: String,
    // Build executor integration
    #[allocative(skip)]
    pub executor: Option<Arc<tokio::sync::Mutex<dyn BuildExecutor>>>,
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
            build_prefix: String::new(), // Empty means install directly to stage/
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
            build_prefix: String::new(), // Empty means install directly to stage/
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

    #[must_use]
    pub fn with_build_prefix(mut self, build_prefix: String) -> Self {
        self.build_prefix = build_prefix;
        self
    }

    /// Add a build step
    pub(crate) fn add_step(&self, step: BuildStep) {
        self.steps.borrow_mut().push(step);
    }
}

impl Display for BuildContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BuildContext(prefix={}, build_prefix={}, jobs={}, name={}, version={})",
            self.prefix, self.build_prefix, self.jobs, self.name, self.version
        )
    }
}

unsafe impl Trace<'_> for BuildContext {
    fn trace(&mut self, _tracer: &starlark::values::Tracer<'_>) {
        // No Value<'v> types to trace in BuildContext
    }
}

#[starlark_value(type = "BuildContext")]
impl<'v> StarlarkValue<'v> for BuildContext {
    fn has_attr(&self, attribute: &str, _heap: &'v Heap) -> bool {
        matches!(
            attribute,
            "PREFIX" | "BUILD_PREFIX" | "JOBS" | "NAME" | "VERSION"
        )
    }

    fn get_attr(&self, attribute: &str, heap: &'v Heap) -> Option<Value<'v>> {
        match attribute {
            "PREFIX" => Some(heap.alloc(&self.prefix)),
            "BUILD_PREFIX" => Some(heap.alloc(&self.build_prefix)),
            "JOBS" => Some(heap.alloc(self.jobs)),
            "NAME" => Some(heap.alloc(&self.name)),
            "VERSION" => Some(heap.alloc(&self.version)),
            _ => None,
        }
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

/// Functions available for BuildContext in Starlark  
#[starlark_module]
#[allow(clippy::unnecessary_wraps)]
pub fn build_context_functions(builder: &mut GlobalsBuilder) {
    /// Fetch a source archive
    fn fetch<'v>(ctx: Value<'v>, url: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::Fetch {
            url: url.to_string(),
        });
        Ok(NoneType)
    }

    /// Fetch a source archive with MD5 validation
    fn fetch_md5<'v>(ctx: Value<'v>, url: &str, md5_hash: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::FetchMd5 {
            url: url.to_string(),
            md5: md5_hash.to_string(),
        });
        Ok(NoneType)
    }

    /// Fetch a source archive with SHA256 validation
    fn fetch_sha256<'v>(ctx: Value<'v>, url: &str, sha256_hash: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::FetchSha256 {
            url: url.to_string(),
            sha256: sha256_hash.to_string(),
        });
        Ok(NoneType)
    }

    /// Fetch a source archive with BLAKE3 validation
    fn fetch_blake3<'v>(ctx: Value<'v>, url: &str, blake3_hash: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::FetchBlake3 {
            url: url.to_string(),
            blake3: blake3_hash.to_string(),
        });
        Ok(NoneType)
    }

    /// Clone a git repository
    fn git<'v>(ctx: Value<'v>, url: &str, ref_: Option<&str>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let git_ref = ref_.unwrap_or("HEAD").to_string();
        build_ctx.add_step(BuildStep::Git {
            url: url.to_string(),
            ref_: git_ref,
        });
        Ok(NoneType)
    }

    /// Apply a patch file
    fn apply_patch<'v>(ctx: Value<'v>, path: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::ApplyPatch {
            path: path.to_string(),
        });
        Ok(NoneType)
    }

    /// Run an arbitrary command (pass command as string or list)
    fn command<'v>(ctx: Value<'v>, cmd: Value<'v>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;
        // Handle both string and list inputs
        let (program, args) = if let Some(s) = cmd.unpack_str() {
            // Single string: split into program and args
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.is_empty() {
                return Err(anyhow::anyhow!("Command cannot be empty"));
            }
            let program = parts[0].to_string();
            let args: Vec<String> = parts[1..].iter().map(ToString::to_string).collect();
            (program, args)
        } else if let Some(list) = starlark::values::list::ListRef::from_value(cmd) {
            // List: first element is program, rest are args
            if list.is_empty() {
                return Err(anyhow::anyhow!("Command list cannot be empty"));
            }
            let mut iter = list.iter();
            let program = iter
                .next()
                .and_then(starlark::values::Value::unpack_str)
                .ok_or_else(|| anyhow::anyhow!("Program name must be a string"))?
                .to_string();
            let args: Vec<String> = iter
                .map(|v| {
                    v.unpack_str()
                        .ok_or_else(|| anyhow::anyhow!("All command arguments must be strings"))
                        .map(std::string::ToString::to_string)
                })
                .collect::<Result<Vec<_>, _>>()?;
            (program, args)
        } else {
            return Err(anyhow::anyhow!(
                "Command must be a string or list of strings"
            ));
        };

        build_ctx.add_step(BuildStep::Command { program, args });
        Ok(NoneType)
    }

    /// Run make install
    fn install<'v>(ctx: Value<'v>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::Install);
        Ok(NoneType)
    }

    /// Detect the build system for the current source
    fn detect_build_system<'v>(ctx: Value<'v>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::DetectBuildSystem);
        Ok(NoneType)
    }

    /// Set the build system to use
    fn set_build_system<'v>(ctx: Value<'v>, name: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx
            .detected_build_system
            .replace(Some(name.to_string()));
        build_ctx.add_step(BuildStep::SetBuildSystem {
            name: name.to_string(),
        });
        Ok(NoneType)
    }

    /// Create a checkpoint for recovery
    fn checkpoint<'v>(ctx: Value<'v>, name: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.checkpoints.borrow_mut().push(name.to_string());
        build_ctx.add_step(BuildStep::Checkpoint {
            name: name.to_string(),
        });
        Ok(NoneType)
    }

    /// Set error handler
    fn on_error<'v>(ctx: Value<'v>, handler: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx
            .error_handlers
            .borrow_mut()
            .push(handler.to_string());
        build_ctx.add_step(BuildStep::OnError {
            handler: handler.to_string(),
        });
        Ok(NoneType)
    }

    /// Allow or disallow network access during build
    fn allow_network<'v>(ctx: Value<'v>, enabled: bool) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.network_allowed.replace(enabled);
        build_ctx.add_step(BuildStep::AllowNetwork { enabled });
        Ok(NoneType)
    }

    /// Set an environment variable
    fn set_env<'v>(ctx: Value<'v>, key: &str, value: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::SetEnv {
            key: key.to_string(),
            value: value.to_string(),
        });
        Ok(NoneType)
    }

    /// Clean up the staging directory after build
    fn cleanup<'v>(ctx: Value<'v>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::Cleanup);
        Ok(NoneType)
    }

    /// Copy source files from recipe directory to build directory
    fn copy<'v>(ctx: Value<'v>, src_path: Option<&str>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::Copy {
            src_path: src_path.map(str::to_string),
        });
        Ok(NoneType)
    }

    /// Apply sps2 recommended compiler flags and environment variables for macOS ARM64
    ///
    /// This function sets optimized compiler flags based on the target architecture and
    /// build system. It's optional but recommended for performance and security.
    ///
    /// Example:
    ///   with_defaults(ctx)  # Apply recommended flags
    ///   fetch(ctx, "https://example.com/source.tar.gz")
    ///   cmake(ctx, [])
    fn with_defaults<'v>(ctx: Value<'v>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.add_step(BuildStep::WithDefaults);
        Ok(NoneType)
    }
}
