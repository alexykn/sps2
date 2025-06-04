//! Parallel execution support for Starlark recipes

use crate::recipe::BuildStep;
use crate::starlark::context::BuildContext;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::none::NoneType;
use starlark::values::{Value, ValueLike};

/// Register parallel execution functions as globals
pub fn register_globals(builder: &mut GlobalsBuilder) {
    parallel_module(builder);
}

/// Parallel execution functions exposed to Starlark
#[starlark_module]
#[allow(clippy::unnecessary_wraps)]
fn parallel_module(builder: &mut GlobalsBuilder) {
    /// Set the parallelism level for builds
    ///
    /// Example:
    /// - set_parallelism(ctx, 4)    # Use 4 parallel jobs
    /// - set_parallelism(ctx, 1)    # Disable parallelism
    fn set_parallelism<'v>(ctx: Value<'v>, jobs: i32) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        if jobs <= 0 {
            return Err(anyhow::anyhow!("Jobs must be a positive integer"));
        }

        let jobs_usize = jobs.try_into().unwrap_or(1);
        build_ctx.parallelism.replace(jobs_usize);
        build_ctx.add_step(BuildStep::SetParallelism { jobs: jobs_usize });
        Ok(NoneType)
    }

    /// Execute steps in parallel
    ///
    /// Example:
    /// ```text
    /// parallel_steps(ctx, lambda: [
    ///     ctx.command("make", "-C", "lib1"),
    ///     ctx.command("make", "-C", "lib2"),
    ///     ctx.command("make", "-C", "lib3")
    /// ])
    /// ```
    fn parallel_steps<'v>(
        ctx: Value<'v>,
        steps_fn: Value<'v>,
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

        // Add a single ParallelSteps containing all the steps
        build_ctx.add_step(BuildStep::ParallelSteps { steps: new_steps });

        Ok(NoneType)
    }

    /// Set resource hints for the build
    ///
    /// Example:
    /// - set_resource_hints(ctx, cpu=4)           # Hint: needs 4 CPU cores
    /// - set_resource_hints(ctx, memory_mb=8192)  # Hint: needs 8GB RAM
    /// - set_resource_hints(ctx, cpu=8, memory_mb=16384)
    fn set_resource_hints<'v>(
        ctx: Value<'v>,
        cpu: Option<i32>,
        memory_mb: Option<i32>,
    ) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;
        let cpu_usize = cpu
            .map(|c| {
                if c <= 0 {
                    return Err(anyhow::anyhow!("CPU cores must be positive"));
                }
                c.try_into()
                    .map_err(|_| anyhow::anyhow!("CPU cores value too large"))
            })
            .transpose()?;

        let memory_usize = memory_mb
            .map(|m| {
                if m <= 0 {
                    return Err(anyhow::anyhow!("Memory must be positive"));
                }
                m.try_into()
                    .map_err(|_| anyhow::anyhow!("Memory value too large"))
            })
            .transpose()?;

        build_ctx.resource_hints.replace((cpu_usize, memory_usize));
        build_ctx.add_step(BuildStep::SetResourceHints {
            cpu: cpu_usize,
            memory_mb: memory_usize,
        });
        Ok(NoneType)
    }
}
