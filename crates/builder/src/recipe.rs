//! Starlark recipe execution and build step management

use crate::events::send_event;
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

/// Execute the Starlark recipe and return dependencies, metadata, and install request status
pub async fn execute_recipe(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
) -> Result<(Vec<String>, Vec<PackageSpec>, RecipeMetadata, bool), Error> {
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

    let mut api = BuilderApi::new(working_dir.clone())?;
    let _result = api.allow_network(config.allow_network);

    // Execute recipe with timeout
    let result = with_optional_timeout(
        execute_recipe_steps(context, &recipe, &mut api, environment),
        config.max_build_time,
        &context.name,
    )
    .await?;

    // Check if install was requested
    let install_requested = api.is_install_requested();

    // Transfer build metadata from API to environment
    for (key, value) in api.build_metadata() {
        environment.set_build_metadata(key.clone(), value.clone());
    }

    Ok((result.0, result.1, result.2, install_requested))
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
        // Fetch operations
        BuildStep::Fetch { url } => {
            api.fetch(url).await?;
        }
        BuildStep::FetchMd5 { url, md5 } => {
            api.fetch_md5(url, md5).await?;
        }
        BuildStep::FetchSha256 { url, sha256 } => {
            api.fetch_sha256(url, sha256).await?;
        }
        BuildStep::FetchBlake3 { url, blake3 } => {
            api.fetch_blake3(url, blake3).await?;
        }
        BuildStep::Extract => {
            api.extract_downloads().await?;
        }
        BuildStep::Git { url, ref_ } => {
            api.git(url, ref_).await?;
        }

        // Build system operations
        step if matches!(
            step,
            BuildStep::Configure { .. }
                | BuildStep::Make { .. }
                | BuildStep::Autotools { .. }
                | BuildStep::Cmake { .. }
                | BuildStep::Meson { .. }
                | BuildStep::Cargo { .. }
                | BuildStep::Go { .. }
                | BuildStep::Python { .. }
                | BuildStep::NodeJs { .. }
        ) =>
        {
            execute_build_system_step(step, api, environment).await?;
        }

        // Basic operations
        BuildStep::Install => {
            api.install(environment).await?;
        }
        BuildStep::ApplyPatch { path } => {
            api.apply_patch(Path::new(path), environment).await?;
        }
        BuildStep::Command { program, args } => {
            execute_command_step(program, args, api, environment).await?;
        }
        BuildStep::SetEnv { key, value } => {
            environment.set_env_var(key.clone(), value.clone())?;
        }
        BuildStep::AllowNetwork { enabled } => {
            let _result = api.allow_network(*enabled);
        }
        BuildStep::Cleanup => {
            cleanup_staging_directory(environment).await?;
        }
        BuildStep::Copy { src_path } => {
            api.copy(src_path.as_deref(), &environment.context).await?;
        }

        // Advanced features
        _ => {
            execute_advanced_step(step, api, environment).await?;
        }
    }

    Ok(())
}

/// Execute build system specific steps
async fn execute_build_system_step(
    step: &BuildStep,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    match step {
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
        BuildStep::Go { args } => {
            api.go(args, environment).await?;
        }
        BuildStep::Python { args } => {
            api.python(args, environment).await?;
        }
        BuildStep::NodeJs { args } => {
            api.nodejs(args, environment).await?;
        }
        _ => unreachable!("Only build system steps should be passed to this function"),
    }
    Ok(())
}

/// Execute advanced features and experimental steps
async fn execute_advanced_step(
    step: &BuildStep,
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    match step {
        // Build system detection
        BuildStep::DetectBuildSystem => {
            // TODO: Implement build system detection
        }
        BuildStep::SetBuildSystem { name: _ } => {
            // TODO: Store build system preference
        }
        // Feature flags
        BuildStep::EnableFeature { name: _ } => {
            // TODO: Enable feature in build context
        }
        BuildStep::DisableFeature { name: _ } => {
            // TODO: Disable feature in build context
        }
        BuildStep::WithFeatures { features: _, steps } => {
            execute_conditional_steps(steps, api, environment).await?;
        }
        // Error recovery
        BuildStep::TryRecover {
            steps,
            recovery_strategy: _,
        } => {
            execute_recovery_steps(steps, api, environment).await?;
        }
        BuildStep::OnError { handler: _ } => {
            // TODO: Register error handler
        }
        BuildStep::Checkpoint { name: _ } => {
            // TODO: Create checkpoint for recovery
        }
        // Cross-compilation
        BuildStep::SetTarget { triple: _ } => {
            // TODO: Configure cross-compilation target
        }
        BuildStep::SetToolchain { name: _, path: _ } => {
            // TODO: Configure toolchain component
        }
        // Parallel execution
        BuildStep::SetParallelism { jobs: _ } => {
            // TODO: Update parallelism level
        }
        BuildStep::ParallelSteps { steps } => {
            execute_parallel_steps(steps, api, environment).await?;
        }
        BuildStep::SetResourceHints {
            cpu: _,
            memory_mb: _,
        } => {
            // TODO: Set resource hints for scheduler
        }
        _ => {
            // This should not happen if the main match is correct
        }
    }
    Ok(())
}

/// Execute a command step with proper DESTDIR handling
async fn execute_command_step(
    program: &str,
    args: &[String],
    api: &BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    // Process arguments to handle DESTDIR properly
    let processed_args: Vec<String> = if program == "make" {
        args.iter()
            .map(|arg| {
                if arg.starts_with("DESTDIR=") {
                    // Always use the absolute staging directory
                    format!("DESTDIR={}", environment.staging_dir().display())
                } else {
                    arg.clone()
                }
            })
            .collect()
    } else {
        args.to_vec()
    };

    let arg_refs: Vec<&str> = processed_args.iter().map(String::as_str).collect();
    environment
        .execute_command(program, &arg_refs, Some(&api.working_dir))
        .await?;
    Ok(())
}

/// Execute conditional steps based on features
async fn execute_conditional_steps(
    steps: &[BuildStep],
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    // TODO: Check features and conditionally execute steps
    // For now, execute all steps unconditionally
    for step in steps {
        Box::pin(execute_build_step(step, api, environment)).await?;
    }
    Ok(())
}

/// Execute steps with recovery strategy
async fn execute_recovery_steps(
    steps: &[BuildStep],
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    // TODO: Execute steps with recovery strategy
    // For now, execute steps normally
    for step in steps {
        Box::pin(execute_build_step(step, api, environment)).await?;
    }
    Ok(())
}

/// Execute steps in parallel
async fn execute_parallel_steps(
    steps: &[BuildStep],
    api: &mut BuilderApi,
    environment: &mut BuildEnvironment,
) -> Result<(), Error> {
    // TODO: Execute steps in parallel
    // For now, execute steps sequentially
    for step in steps {
        Box::pin(execute_build_step(step, api, environment)).await?;
    }
    Ok(())
}

/// Clean up the staging directory for the current package
async fn cleanup_staging_directory(environment: &BuildEnvironment) -> Result<(), Error> {
    let staging_dir = environment.staging_dir();
    let source_dir = environment.build_prefix().join("src");

    // Clean staging directory if it exists
    if staging_dir.exists() {
        // Remove all contents but keep the directory itself
        let mut entries = fs::read_dir(&staging_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path).await?;
            } else {
                fs::remove_file(&path).await?;
            }
        }

        // Send event about cleanup
        send_event(
            environment.context(),
            Event::DebugLog {
                message: format!("Cleaned staging directory: {}", staging_dir.display()),
                context: std::collections::HashMap::new(),
            },
        );
    }

    // Clean source directory if it exists
    if source_dir.exists() {
        // Remove all contents but keep the directory itself
        let mut entries = fs::read_dir(&source_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path).await?;
            } else {
                fs::remove_file(&path).await?;
            }
        }

        // Send event about cleanup
        send_event(
            environment.context(),
            Event::DebugLog {
                message: format!("Cleaned source directory: {}", source_dir.display()),
                context: std::collections::HashMap::new(),
            },
        );
    }

    Ok(())
}
