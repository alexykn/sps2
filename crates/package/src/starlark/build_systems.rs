//! Build system methods for Starlark recipes

use crate::recipe::BuildStep;
use crate::starlark::context::BuildContext;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::list::ListRef;
use starlark::values::none::NoneType;
use starlark::values::{Value, ValueLike};

/// Register build system functions as globals
pub fn register_globals(builder: &mut GlobalsBuilder) {
    build_systems_module(builder);
}

/// Build system functions exposed to Starlark
#[starlark_module]
#[allow(clippy::unnecessary_wraps)]
fn build_systems_module(builder: &mut GlobalsBuilder) {
    /// Run make with optional arguments
    ///
    /// Examples:
    /// - make(ctx, [])                    # runs 'make'
    /// - make(ctx, ["-j4"])              # runs 'make -j4'
    /// - make(ctx, ["install", "PREFIX=/opt"])  # runs 'make install PREFIX=/opt'
    fn make<'v>(ctx: Value<'v>, args: Option<Value<'v>>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let mut arg_vec = Vec::new();

        if let Some(args_value) = args {
            if let Some(list) = ListRef::from_value(args_value) {
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        arg_vec.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("All arguments must be strings"));
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Arguments must be a list of strings"));
            }
        }

        build_ctx.add_step(BuildStep::Make { args: arg_vec });
        Ok(NoneType)
    }

    /// Run configure script with optional arguments
    ///
    /// Examples:
    /// - configure(ctx, [])                     # runs './configure'
    /// - configure(ctx, ["--prefix=/opt"])      # runs './configure --prefix=/opt'
    /// - configure(ctx, ["--enable-foo", "--disable-bar"])
    fn configure<'v>(ctx: Value<'v>, args: Option<Value<'v>>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let mut arg_vec = Vec::new();

        if let Some(args_value) = args {
            if let Some(list) = ListRef::from_value(args_value) {
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        arg_vec.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("All arguments must be strings"));
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Arguments must be a list of strings"));
            }
        }

        build_ctx.add_step(BuildStep::Configure { args: arg_vec });
        Ok(NoneType)
    }

    /// Run autotools build (configure && make && make install)
    ///
    /// Examples:
    /// - autotools(ctx, [])                     # standard autotools build
    /// - autotools(ctx, ["--prefix=/opt"])      # with configure args
    fn autotools<'v>(ctx: Value<'v>, args: Option<Value<'v>>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let mut arg_vec = Vec::new();

        if let Some(args_value) = args {
            if let Some(list) = ListRef::from_value(args_value) {
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        arg_vec.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("All arguments must be strings"));
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Arguments must be a list of strings"));
            }
        }

        build_ctx.add_step(BuildStep::Autotools { args: arg_vec });
        Ok(NoneType)
    }

    /// Run CMake build
    ///
    /// Examples:
    /// - cmake(ctx, ["-DCMAKE_INSTALL_PREFIX=/opt"])
    /// - cmake(ctx, ["-DCMAKE_BUILD_TYPE=Release", "-DWITH_SSL=ON"])
    fn cmake<'v>(ctx: Value<'v>, args: Option<Value<'v>>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let mut arg_vec = Vec::new();

        if let Some(args_value) = args {
            if let Some(list) = ListRef::from_value(args_value) {
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        arg_vec.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("All arguments must be strings"));
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Arguments must be a list of strings"));
            }
        }

        build_ctx.add_step(BuildStep::Cmake { args: arg_vec });
        Ok(NoneType)
    }

    /// Run Meson build
    ///
    /// Examples:
    /// - meson(ctx, ["setup", "build", "--prefix=/opt"])
    /// - meson(ctx, ["compile", "-C", "build"])
    fn meson<'v>(ctx: Value<'v>, args: Option<Value<'v>>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let mut arg_vec = Vec::new();

        if let Some(args_value) = args {
            if let Some(list) = ListRef::from_value(args_value) {
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        arg_vec.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("All arguments must be strings"));
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Arguments must be a list of strings"));
            }
        }

        build_ctx.add_step(BuildStep::Meson { args: arg_vec });
        Ok(NoneType)
    }

    /// Run Cargo build
    ///
    /// Examples:
    /// - cargo(ctx, ["build", "--release"])
    /// - cargo(ctx, ["install", "--path", ".", "--root", "/opt"])
    fn cargo<'v>(ctx: Value<'v>, args: Option<Value<'v>>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let mut arg_vec = Vec::new();

        if let Some(args_value) = args {
            if let Some(list) = ListRef::from_value(args_value) {
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        arg_vec.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("All arguments must be strings"));
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Arguments must be a list of strings"));
            }
        }

        build_ctx.add_step(BuildStep::Cargo { args: arg_vec });
        Ok(NoneType)
    }

    /// Run Go build
    ///
    /// Examples:
    /// - go(ctx, ["build", "-o", "myapp"])
    /// - go(ctx, ["test", "./..."])
    /// - go(ctx, ["mod", "vendor"])
    fn go<'v>(ctx: Value<'v>, args: Option<Value<'v>>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let mut arg_vec = Vec::new();

        if let Some(args_value) = args {
            if let Some(list) = ListRef::from_value(args_value) {
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        arg_vec.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("All arguments must be strings"));
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Arguments must be a list of strings"));
            }
        }

        build_ctx.add_step(BuildStep::Go { args: arg_vec });
        Ok(NoneType)
    }

    /// Run Python build
    ///
    /// Examples:
    /// - python(ctx, ["setup.py", "build"])
    /// - python(ctx, ["setup.py", "install", "--prefix=/opt"])
    /// - python(ctx, ["-m", "pip", "install", "."])
    fn python<'v>(ctx: Value<'v>, args: Option<Value<'v>>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let mut arg_vec = Vec::new();

        if let Some(args_value) = args {
            if let Some(list) = ListRef::from_value(args_value) {
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        arg_vec.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("All arguments must be strings"));
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Arguments must be a list of strings"));
            }
        }

        build_ctx.add_step(BuildStep::Python { args: arg_vec });
        Ok(NoneType)
    }

    /// Run Node.js/npm build
    ///
    /// Examples:
    /// - nodejs(ctx, ["npm", "install"])
    /// - nodejs(ctx, ["npm", "run", "build"])
    /// - nodejs(ctx, ["yarn", "install"])
    /// - nodejs(ctx, ["pnpm", "install"])
    fn nodejs<'v>(ctx: Value<'v>, args: Option<Value<'v>>) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        let mut arg_vec = Vec::new();

        if let Some(args_value) = args {
            if let Some(list) = ListRef::from_value(args_value) {
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        arg_vec.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("All arguments must be strings"));
                    }
                }
            } else {
                return Err(anyhow::anyhow!("Arguments must be a list of strings"));
            }
        }

        build_ctx.add_step(BuildStep::NodeJs { args: arg_vec });
        Ok(NoneType)
    }
}
