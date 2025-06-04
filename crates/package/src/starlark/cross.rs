//! Cross-compilation support for Starlark recipes

use crate::recipe::BuildStep;
use crate::starlark::context::BuildContext;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::none::NoneType;
use starlark::values::{Value, ValueLike};

/// Register cross-compilation functions as globals
pub fn register_globals(builder: &mut GlobalsBuilder) {
    cross_module(builder);
}

/// Cross-compilation functions exposed to Starlark
#[starlark_module]
#[allow(clippy::unnecessary_wraps)]
fn cross_module(builder: &mut GlobalsBuilder) {
    /// Set the target triple for cross-compilation
    ///
    /// Example:
    /// - set_target(ctx, "aarch64-apple-darwin")
    /// - set_target(ctx, "x86_64-unknown-linux-gnu")
    fn set_target<'v>(ctx: Value<'v>, triple: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx.target_triple.replace(Some(triple.to_string()));
        build_ctx.add_step(BuildStep::SetTarget {
            triple: triple.to_string(),
        });
        Ok(NoneType)
    }

    /// Set a toolchain component for cross-compilation
    ///
    /// Example:
    /// - set_toolchain(ctx, "CC", "aarch64-apple-darwin-gcc")
    /// - set_toolchain(ctx, "CXX", "aarch64-apple-darwin-g++")
    /// - set_toolchain(ctx, "AR", "aarch64-apple-darwin-ar")
    fn set_toolchain<'v>(ctx: Value<'v>, name: &str, path: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx
            .toolchain
            .borrow_mut()
            .insert(name.to_string(), path.to_string());
        build_ctx.add_step(BuildStep::SetToolchain {
            name: name.to_string(),
            path: path.to_string(),
        });
        Ok(NoneType)
    }

    /// Try to recover from build errors with a specific strategy
    ///
    /// Example:
    /// ```text
    /// try_recover(ctx, "retry", lambda: [
    ///     ctx.configure("--host=aarch64-apple-darwin"),
    ///     ctx.make()
    /// ])
    /// ```
    fn try_recover<'v>(
        ctx: Value<'v>,
        recovery_strategy: &str,
        steps_fn: starlark::values::Value<'v>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        // Capture current step count
        let start_idx = build_ctx.steps.borrow().len();

        // Execute the callback to collect steps
        eval.eval_function(steps_fn, &[], &[])
            .map_err(|e| anyhow::anyhow!(e))?;

        // Extract the steps that were added
        let all_steps = build_ctx.steps.borrow();
        let new_steps: Vec<BuildStep> = all_steps[start_idx..].to_vec();
        drop(all_steps);

        // Remove the individually added steps
        build_ctx.steps.borrow_mut().truncate(start_idx);

        // Add a single TryRecover containing all the steps
        build_ctx.add_step(BuildStep::TryRecover {
            steps: new_steps,
            recovery_strategy: recovery_strategy.to_string(),
        });

        Ok(NoneType)
    }
}
