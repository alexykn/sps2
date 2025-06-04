//! Starlark recipe execution and build step management

use crate::events::send_event;
use crate::fileops::copy_source_files;
use crate::timeout_utils::with_optional_timeout;
use crate::{BuildConfig, BuildContext, BuildEnvironment, BuilderApi};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use sps2_package::{
    execute_recipe as package_execute_recipe, load_recipe, BuildStep, RecipeMetadata,
};
use sps2_types::package::PackageSpec;
use std::path::Path;
use tokio::fs;

/// Execute the Starlark recipe and return dependencies and metadata
pub async fn execute_recipe(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
) -> Result<(Vec<String>, Vec<PackageSpec>, RecipeMetadata), Error> {
    // Read recipe file
    let _recipe_content = fs::read_to_string(&context.recipe_path)
        .await
        .map_err(|e| BuildError::RecipeError {
            message: format!(
                "failed to read recipe {}: {e}",
                context.recipe_path.display()
            ),
        })?;

    // Parse the recipe
    let recipe = load_recipe(&context.recipe_path).await?;

    // Create builder API
    let working_dir = environment.build_prefix().join("src");
    fs::create_dir_all(&working_dir).await?;

    // Copy source files from recipe directory to working directory
    let recipe_dir = context
        .recipe_path
        .parent()
        .ok_or_else(|| BuildError::RecipeError {
            message: "Invalid recipe path".to_string(),
        })?;

    copy_source_files(recipe_dir, &working_dir, context).await?;

    let mut api = BuilderApi::new(working_dir.clone())?;
    let _result = api.allow_network(config.allow_network);

    // Execute recipe with timeout
    let result = with_optional_timeout(
        execute_recipe_steps(context, &recipe, &mut api, environment),
        config.max_build_time,
        &context.name,
    )
    .await?;

    Ok(result)
}

/// Execute recipe steps
async fn execute_recipe_steps(
    context: &BuildContext,
    recipe: &sps2_package::Recipe,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(Vec<String>, Vec<PackageSpec>, RecipeMetadata), Error> {
    // Execute the recipe to get metadata
    let recipe_result = package_execute_recipe(recipe)?;

    // Extract runtime dependencies as strings
    let runtime_deps: Vec<String> = recipe_result.metadata.runtime_deps.clone();

    // Extract build dependencies as PackageSpec
    let build_deps: Vec<PackageSpec> = recipe_result
        .metadata
        .build_deps
        .iter()
        .map(|dep| PackageSpec::parse(dep))
        .collect::<Result<Vec<_>, _>>()?;

    // Execute build steps
    for step in &recipe_result.build_steps {
        send_event(
            context,
            Event::BuildStepStarted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            },
        );

        execute_build_step(step, api, environment).await?;

        send_event(
            context,
            Event::BuildStepCompleted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            },
        );
    }

    Ok((runtime_deps, build_deps, recipe_result.metadata.clone()))
}

/// Execute a single build step
async fn execute_build_step(
    step: &BuildStep,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    match step {
        BuildStep::Fetch { url, blake3 } => {
            api.fetch(url, blake3).await?;
        }
        BuildStep::Configure { args } => {
            api.configure(args, environment).await?;
        }
        BuildStep::Make { args } => {
            api.make(args, environment).await?;
        }
        BuildStep::Autotools { args } => {
            api.autotools(args, environment).await?;
        }
        BuildStep::Cmake { args } => {
            api.cmake(args, environment).await?;
        }
        BuildStep::Meson { args } => {
            api.meson(args, environment).await?;
        }
        BuildStep::Cargo { args } => {
            api.cargo(args, environment).await?;
        }
        BuildStep::Install => {
            api.install(environment).await?;
        }
        BuildStep::ApplyPatch { path } => {
            api.apply_patch(Path::new(path), environment).await?;
        }
        BuildStep::Command { program, args } => {
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            environment
                .execute_command(program, &arg_refs, None)
                .await?;
        }
        BuildStep::SetEnv { key, value } => {
            environment.set_env_var(key.clone(), value.clone())?;
        }
        BuildStep::AllowNetwork { enabled } => {
            let _result = api.allow_network(*enabled);
        }
        // New build system detection
        BuildStep::DetectBuildSystem => {
            // TODO: Implement build system detection
            // For now, just log it as a no-op
        }
        BuildStep::SetBuildSystem { name: _ } => {
            // TODO: Store build system preference
            // For now, just log it as a no-op
        }
        // Feature flags
        BuildStep::EnableFeature { name: _ } => {
            // TODO: Enable feature in build context
            // For now, just log it as a no-op
        }
        BuildStep::DisableFeature { name: _ } => {
            // TODO: Disable feature in build context
            // For now, just log it as a no-op
        }
        BuildStep::WithFeatures { features: _, steps } => {
            // TODO: Check features and conditionally execute steps
            // For now, execute all steps unconditionally
            for step in steps {
                Box::pin(execute_build_step(step, api, environment)).await?;
            }
        }
        // Error recovery
        BuildStep::TryRecover {
            steps,
            recovery_strategy: _,
        } => {
            // TODO: Execute steps with recovery strategy
            // For now, execute steps normally
            for step in steps {
                Box::pin(execute_build_step(step, api, environment)).await?;
            }
        }
        BuildStep::OnError { handler: _ } => {
            // TODO: Register error handler
            // For now, just log it as a no-op
        }
        BuildStep::Checkpoint { name: _ } => {
            // TODO: Create checkpoint for recovery
            // For now, just log it as a no-op
        }
        // Cross-compilation
        BuildStep::SetTarget { triple: _ } => {
            // TODO: Configure cross-compilation target
            // For now, just log it as a no-op
        }
        BuildStep::SetToolchain { name: _, path: _ } => {
            // TODO: Configure toolchain component
            // For now, just log it as a no-op
        }
        // Parallel execution
        BuildStep::SetParallelism { jobs: _ } => {
            // TODO: Update parallelism level
            // For now, just log it as a no-op
        }
        BuildStep::ParallelSteps { steps } => {
            // TODO: Execute steps in parallel
            // For now, execute steps sequentially
            for step in steps {
                Box::pin(execute_build_step(step, api, environment)).await?;
            }
        }
        BuildStep::SetResourceHints {
            cpu: _,
            memory_mb: _,
        } => {
            // TODO: Set resource hints for scheduler
            // For now, just log it as a no-op
        }
    }

    Ok(())
}
