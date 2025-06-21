//! Staged execution implementation for proper build ordering

use crate::build_plan::{BuildPlan, EnvironmentConfig};
use crate::environment::BuildEnvironment;
use crate::events::send_event;
use crate::recipe::{execute_build_step, execute_build_steps_list};
use crate::yaml::{parse_yaml_recipe, RecipeMetadata};
use crate::{BuildConfig, BuildContext, BuilderApi};
use sps2_errors::Error;
use sps2_events::Event;
use std::collections::HashMap;
use tokio::fs;

/// Execute a build using staged execution model
pub async fn execute_staged_build(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
) -> Result<
    (
        Vec<String>,
        Vec<sps2_types::package::PackageSpec>,
        RecipeMetadata,
        bool,
    ),
    Error,
> {
    // Stage 0: Parse and analyze recipe
    send_event(
        context,
        Event::OperationStarted {
            operation: "Analyzing recipe".to_string(),
        },
    );

    let yaml_recipe = parse_yaml_recipe(&context.recipe_path).await?;
    let build_plan = BuildPlan::from_yaml(&yaml_recipe);

    send_event(
        context,
        Event::OperationCompleted {
            operation: "Recipe analysis complete".to_string(),
            success: true,
        },
    );

    // Stage 1: Apply environment configuration
    apply_environment_config(context, environment, &build_plan.environment).await?;

    // Stage 2: Execute source operations
    execute_source_stage(config, context, environment, &build_plan).await?;

    // Stage 3: Execute build operations
    execute_build_stage(config, context, environment, &build_plan).await?;

    // Stage 4: Execute post-processing operations
    execute_post_stage(config, context, environment, &build_plan).await?;

    // Extract dependencies
    let runtime_deps = build_plan.metadata.runtime_deps.clone();
    let build_deps: Vec<sps2_types::package::PackageSpec> = build_plan
        .metadata
        .build_deps
        .iter()
        .map(|dep| sps2_types::package::PackageSpec::parse(dep))
        .collect::<Result<Vec<_>, _>>()?;

    Ok((
        runtime_deps,
        build_deps,
        build_plan.metadata,
        build_plan.auto_install,
    ))
}

/// Apply environment configuration before any build steps
async fn apply_environment_config(
    context: &BuildContext,
    environment: &mut BuildEnvironment,
    config: &EnvironmentConfig,
) -> Result<(), Error> {
    send_event(
        context,
        Event::OperationStarted {
            operation: "Configuring build environment".to_string(),
        },
    );

    // Apply isolation level from recipe
    if config.isolation != environment.isolation_level() {
        send_event(
            context,
            Event::DebugLog {
                message: format!("Applying isolation level {} from recipe", config.isolation),
                context: HashMap::new(),
            },
        );

        environment.set_isolation_level_from_recipe(config.isolation);
        environment
            .apply_isolation_level(
                config.isolation,
                config.network,
                context.event_sender.as_ref(),
            )
            .await?;

        // Verify isolation (skip for None)
        if config.isolation != crate::environment::IsolationLevel::None {
            environment.verify_isolation()?;
        }
    }

    // Apply compiler defaults if requested
    if config.defaults {
        send_event(
            context,
            Event::BuildStepStarted {
                step: "WithDefaults".to_string(),
                package: context.name.clone(),
            },
        );

        environment.apply_default_compiler_flags();

        send_event(
            context,
            Event::BuildStepCompleted {
                step: "WithDefaults".to_string(),
                package: context.name.clone(),
            },
        );
    }

    // Set environment variables
    for (key, value) in &config.variables {
        environment.set_env_var(key.clone(), value.clone())?;
    }

    send_event(
        context,
        Event::OperationCompleted {
            operation: "Build environment configured".to_string(),
            success: true,
        },
    );

    Ok(())
}

/// Execute source acquisition stage
async fn execute_source_stage(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
    build_plan: &BuildPlan,
) -> Result<(), Error> {
    if build_plan.source_steps.is_empty() {
        return Ok(());
    }

    send_event(
        context,
        Event::OperationStarted {
            operation: "Acquiring sources".to_string(),
        },
    );

    // Create working directory
    let working_dir = environment.build_prefix().join("src");
    fs::create_dir_all(&working_dir).await?;

    // Create builder API
    let mut api = BuilderApi::new(working_dir.clone())?;
    let _result = api.allow_network(config.allow_network);

    // Clean staging area first
    send_event(
        context,
        Event::BuildStepStarted {
            step: "Cleanup".to_string(),
            package: context.name.clone(),
        },
    );

    execute_build_step(&crate::yaml::BuildStep::Cleanup, &mut api, environment).await?;

    send_event(
        context,
        Event::BuildStepCompleted {
            step: "Cleanup".to_string(),
            package: context.name.clone(),
        },
    );

    // Execute source steps
    for step in &build_plan.source_steps {
        send_event(
            context,
            Event::BuildStepStarted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            },
        );

        execute_build_step(step, &mut api, environment).await?;

        send_event(
            context,
            Event::BuildStepCompleted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            },
        );
    }

    send_event(
        context,
        Event::OperationCompleted {
            operation: "Sources acquired".to_string(),
            success: true,
        },
    );

    Ok(())
}

/// Execute build stage
async fn execute_build_stage(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
    build_plan: &BuildPlan,
) -> Result<(), Error> {
    if build_plan.build_steps.is_empty() {
        return Ok(());
    }

    send_event(
        context,
        Event::OperationStarted {
            operation: "Building package".to_string(),
        },
    );

    // Get working directory
    let working_dir = environment.build_prefix().join("src");

    // Create builder API
    let mut api = BuilderApi::new(working_dir)?;
    let _result = api.allow_network(config.allow_network);

    // Execute build steps with timeout
    crate::timeout_utils::with_optional_timeout(
        execute_build_steps_list(context, &build_plan.build_steps, &mut api, environment),
        config.max_build_time,
        &context.name,
    )
    .await?;

    // Transfer build metadata from API to environment
    for (key, value) in api.build_metadata() {
        environment.set_build_metadata(key.clone(), value.clone());
    }

    send_event(
        context,
        Event::OperationCompleted {
            operation: "Build complete".to_string(),
            success: true,
        },
    );

    Ok(())
}

/// Execute post-processing stage
async fn execute_post_stage(
    _config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
    build_plan: &BuildPlan,
) -> Result<(), Error> {
    if build_plan.post_steps.is_empty() {
        return Ok(());
    }

    send_event(
        context,
        Event::OperationStarted {
            operation: "Post-processing".to_string(),
        },
    );

    // Get working directory
    let working_dir = environment.build_prefix().join("src");

    // Create builder API
    let mut api = BuilderApi::new(working_dir)?;

    // Execute post-processing steps
    for step in &build_plan.post_steps {
        send_event(
            context,
            Event::BuildStepStarted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            },
        );

        execute_build_step(step, &mut api, environment).await?;

        send_event(
            context,
            Event::BuildStepCompleted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            },
        );
    }

    send_event(
        context,
        Event::OperationCompleted {
            operation: "Post-processing complete".to_string(),
            success: true,
        },
    );

    Ok(())
}
