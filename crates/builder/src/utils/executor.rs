//! Staged execution implementation for proper build ordering

use crate::build_plan::{BuildPlan, EnvironmentConfig};
use crate::environment::BuildEnvironment;
use crate::recipe::parser::parse_yaml_recipe;
use crate::security::SecurityContext;
use crate::stages::executors::{
    execute_build_commands_list_with_security, execute_post_step_with_security, execute_source_step,
};
use crate::utils::events::send_event;
use crate::yaml::RecipeMetadata;
use crate::{BuildConfig, BuildContext, BuilderApi};
use sps2_errors::Error;
use sps2_events::{AppEvent, GeneralEvent};
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
        sps2_types::QaPipelineOverride,
    ),
    Error,
> {
    // Stage 0: Parse and analyze recipe
    let yaml_recipe = parse_yaml_recipe(&context.recipe_path).await?;
    let build_plan = BuildPlan::from_yaml(
        &yaml_recipe,
        &context.recipe_path,
        config.sps2_config.as_ref(),
    )?;

    send_event(
        context,
        AppEvent::General(GeneralEvent::debug("Recipe analysis completed")),
    );

    // Create security context for the build
    let build_root = environment.build_prefix().to_path_buf();
    let mut initial_vars = HashMap::new();

    // Add build-specific variables
    initial_vars.insert("NAME".to_string(), context.name.clone());
    initial_vars.insert("VERSION".to_string(), context.version.to_string());

    let mut security_context = SecurityContext::new(build_root, initial_vars);

    // Stage 1: Apply environment configuration
    apply_environment_config(context, environment, &build_plan.environment).await?;

    // Stage 2: Execute source operations
    execute_source_stage(config, context, environment, &build_plan).await?;

    // Stage 3: Execute build operations (with security context)
    execute_build_stage_with_security(
        config,
        context,
        environment,
        &build_plan,
        &mut security_context,
    )
    .await?;

    // Stage 4: Execute post-processing operations (with security context)
    execute_post_stage_with_security(
        config,
        context,
        environment,
        &build_plan,
        &mut security_context,
    )
    .await?;

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
        build_plan.qa_pipeline,
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
        AppEvent::General(GeneralEvent::debug("Configuring build environment")),
    );

    // Apply isolation level from recipe
    if config.isolation != environment.isolation_level() {
        send_event(
            context,
            AppEvent::General(GeneralEvent::debug(format!(
                "Applying isolation level {} from recipe",
                config.isolation
            ))),
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
            AppEvent::General(GeneralEvent::debug("Applying compiler defaults")),
        );

        environment.apply_default_compiler_flags();
    }

    // Set environment variables
    for (key, value) in &config.variables {
        environment.set_env_var(key.clone(), value.clone())?;
    }

    send_event(
        context,
        AppEvent::General(GeneralEvent::debug("Environment configuration complete")),
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
        AppEvent::General(GeneralEvent::debug("Acquiring sources")),
    );

    // Create working directory
    let working_dir = environment.build_prefix().join("src");
    fs::create_dir_all(&working_dir).await?;

    // Create builder API
    let mut api = BuilderApi::new(working_dir.clone(), config.resources.clone())?;
    // Source stage always allows network for fetching
    let _result = api.allow_network(true);

    // Clean staging area first
    send_event(
        context,
        AppEvent::General(GeneralEvent::debug("Cleaning staging area")),
    );

    // Cleanup is handled as the first source step
    execute_source_step(&crate::stages::SourceStep::Cleanup, &mut api, environment).await?;

    // Execute source steps
    for step in &build_plan.source_steps {
        execute_source_step(step, &mut api, environment).await?;

        // Command completed - duration tracking removed as per architectural decision
    }

    send_event(
        context,
        AppEvent::General(GeneralEvent::debug("Source acquisition completed")),
    );

    Ok(())
}

/// Execute build stage with security context
async fn execute_build_stage_with_security(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
    build_plan: &BuildPlan,
    security_context: &mut SecurityContext,
) -> Result<(), Error> {
    if build_plan.build_steps.is_empty() {
        return Ok(());
    }

    send_event(
        context,
        AppEvent::General(GeneralEvent::debug("Building package")),
    );

    // Get working directory
    let working_dir = environment.build_prefix().join("src");

    // Update security context to reflect the actual working directory
    security_context.set_current_dir(working_dir.clone());

    // Create builder API
    let mut api = BuilderApi::new(working_dir, config.resources.clone())?;
    // Use network setting from YAML recipe's environment config
    let _result = api.allow_network(build_plan.environment.network);

    // Execute build steps with timeout and security context
    crate::utils::timeout::with_optional_timeout(
        execute_build_commands_list_with_security(
            context,
            &build_plan.build_steps,
            &mut api,
            environment,
            security_context,
            config.sps2_config.as_ref(),
        ),
        config.max_build_time(),
        &context.name,
    )
    .await?;

    // Transfer build metadata from API to environment
    for (key, value) in api.build_metadata() {
        environment.set_build_metadata(key.clone(), value.clone());
    }

    send_event(
        context,
        AppEvent::General(GeneralEvent::debug("Package build completed")),
    );

    Ok(())
}

/// Execute post-processing stage with security context
async fn execute_post_stage_with_security(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
    build_plan: &BuildPlan,
    security_context: &mut SecurityContext,
) -> Result<(), Error> {
    if build_plan.post_steps.is_empty() {
        return Ok(());
    }

    send_event(
        context,
        AppEvent::General(GeneralEvent::debug("Post-processing pipeline")),
    );

    // Get working directory
    let working_dir = environment.build_prefix().join("src");

    // Update security context to reflect the actual working directory
    security_context.set_current_dir(working_dir.clone());

    // Create builder API
    let mut api = BuilderApi::new(working_dir, config.resources.clone())?;

    // Execute post-processing steps
    for step in &build_plan.post_steps {
        execute_post_step_with_security(
            step,
            &mut api,
            environment,
            security_context,
            config.sps2_config.as_ref(),
        )
        .await?;

        // Command completed - duration tracking removed as per architectural decision
    }

    send_event(
        context,
        AppEvent::General(GeneralEvent::debug("Post-processing completed")),
    );

    Ok(())
}
