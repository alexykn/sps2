//! Pack command implementation
//!
//! Provides standalone packaging functionality without requiring a full rebuild.
//! Always runs post-processing steps and QA pipeline by default, matching build command behavior.

use crate::OpsCtx;
use sps2_builder::{
    artifact_qa::run_quality_pipeline, create_and_sign_package, execute_post_step_with_security,
    generate_sbom_and_manifest, parse_yaml_recipe, BuildCommand, BuildConfig, BuildContext,
    BuildEnvironment, BuildPlan, BuilderApi, RecipeMetadata, SecurityContext, YamlRecipe,
};
use sps2_errors::{Error, OpsError};
use sps2_events::Event;
use sps2_manifest::Manifest;
use sps2_types::{BuildReport, Version};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Pack a recipe from its staging directory with post-processing and QA pipeline
///
/// This is the default pack behavior, matching the build command.
/// Runs post-processing steps, auto-detects QA pipeline from build systems,
/// and allows QA pipeline override from recipe.
///
/// # Errors
///
/// Returns an error if:
/// - Recipe file doesn't exist or has invalid extension  
/// - Staging directory doesn't exist or is empty
/// - Post-step execution fails
/// - QA pipeline fails
/// - Packaging process fails
pub async fn pack_from_recipe(
    ctx: &OpsCtx,
    recipe_path: &Path,
    output_dir: Option<&Path>,
) -> Result<BuildReport, Error> {
    pack_from_recipe_impl(ctx, recipe_path, output_dir, true).await
}

/// Pack a recipe from its staging directory without post-processing or QA pipeline
///
/// This skips all post-processing steps and QA validation.
/// Use --no-post flag to enable this mode.
///
/// # Errors
///
/// Returns an error if:
/// - Recipe file doesn't exist or has invalid extension
/// - Staging directory doesn't exist or is empty
/// - Packaging process fails
pub async fn pack_from_recipe_no_post(
    ctx: &OpsCtx,
    recipe_path: &Path,
    output_dir: Option<&Path>,
) -> Result<BuildReport, Error> {
    pack_from_recipe_impl(ctx, recipe_path, output_dir, false).await
}

/// Pack a directory directly, skipping all post-processing
///
/// This is a power-user feature that packages a directory as-is.
/// It requires a manifest file and optionally accepts an SBOM.
///
/// # Errors
///
/// Returns an error if:
/// - Directory to package does not exist or is empty
/// - Manifest file does not exist or is invalid
/// - SBOM file (if provided) does not exist
/// - Packaging process fails
pub async fn pack_from_directory(
    ctx: &OpsCtx,
    directory: &Path,
    manifest_path: &Path,
    sbom_path: Option<&Path>,
    output_dir: Option<&Path>,
) -> Result<BuildReport, Error> {
    let start = Instant::now();

    ctx.tx
        .send(Event::OperationStarted {
            operation: "Packing from directory".to_string(),
        })
        .map_err(|_| OpsError::EventChannelClosed)?;

    // Validate the directory to be packaged
    validate_staging_directory(directory, "directory", &Version::new(0, 0, 0))?;

    // Load the manifest to get package metadata
    let manifest = Manifest::from_file(manifest_path).await?;
    let package_name = manifest.package.name.clone();
    let package_version = manifest.version()?;

    // Create a minimal build context
    let output_path = determine_output_path(output_dir, &package_name, &package_version);
    let build_context = BuildContext::new(
        package_name.clone(),
        package_version.clone(),
        manifest_path.to_path_buf(), // Use manifest path as a stand-in for recipe
        output_path.parent().unwrap_or(directory).to_path_buf(),
    )
    .with_revision(manifest.package.revision)
    .with_event_sender(ctx.tx.clone());

    // Create a minimal build environment pointing to the specified directory
    let mut environment = BuildEnvironment::new(build_context.clone(), directory)?;
    environment.set_staging_dir(directory.to_path_buf());

    // Create a minimal build config
    let build_config = BuildConfig::default();

    // Prepare SBOM files if provided
    let _sbom_files = if let Some(path) = sbom_path {
        sps2_builder::SbomFiles {
            spdx_path: Some(path.to_path_buf()),
            cyclonedx_path: None,
            ..Default::default()
        }
    } else {
        sps2_builder::SbomFiles::default()
    };

    // Create and sign the package
    let package_path =
        create_and_sign_package(&build_config, &build_context, &environment, manifest).await?;

    let duration = start.elapsed();

    ctx.tx
        .send(Event::OperationCompleted {
            operation: format!("Packed {package_name} v{package_version} from directory"),
            success: true,
        })
        .map_err(|_| OpsError::EventChannelClosed)?;

    Ok(BuildReport {
        package: package_name,
        version: package_version,
        output_path: package_path,
        duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        sbom_generated: sbom_path.is_some(),
    })
}

/// Internal implementation for recipe-based packaging
async fn pack_from_recipe_impl(
    ctx: &OpsCtx,
    recipe_path: &Path,
    output_dir: Option<&Path>,
    execute_post: bool,
) -> Result<BuildReport, Error> {
    let start = Instant::now();

    // Validate recipe file
    validate_recipe_file(recipe_path)?;

    ctx.tx
        .send(Event::OperationStarted {
            operation: format!(
                "Packing from recipe{}",
                if execute_post {
                    " (with post steps)"
                } else {
                    ""
                }
            ),
        })
        .map_err(|_| OpsError::EventChannelClosed)?;

    // Parse recipe to get package metadata
    let yaml_recipe = parse_yaml_recipe(recipe_path).await?;
    let package_name = yaml_recipe.metadata.name.clone();
    let package_version = Version::parse(&yaml_recipe.metadata.version)?;

    // Construct expected staging directory path
    let build_root = PathBuf::from("/opt/pm/build");
    let staging_dir = build_root
        .join(&package_name)
        .join(package_version.to_string())
        .join("stage");

    // Validate staging directory exists and has content
    validate_staging_directory(&staging_dir, &package_name, &package_version)?;

    // Create build context for packaging (same as build command)
    let output_path = determine_output_path(output_dir, &package_name, &package_version);
    let build_context = BuildContext::new(
        package_name.clone(),
        package_version.clone(),
        recipe_path.to_path_buf(),
        output_path.parent().unwrap_or(&build_root).to_path_buf(),
    )
    .with_revision(1)
    .with_event_sender(ctx.tx.clone());

    // Create build environment pointing to existing staging directory
    let mut environment = BuildEnvironment::new(build_context.clone(), &build_root)?;

    // If post steps are requested, execute them (same as build command)
    if execute_post {
        // Detect build systems from recipe build steps for QA pipeline
        let build_plan = BuildPlan::from_yaml(&yaml_recipe, recipe_path, None)?;
        let detected_build_systems = detect_build_systems_from_steps(&build_plan.build_steps);

        // Track detected build systems in environment for QA pipeline
        for build_system in &detected_build_systems {
            environment.record_build_system(build_system);
        }

        execute_post_steps(&build_context, &mut environment, &yaml_recipe).await?;

        // Run QA pipeline (same as build command)
        let qa_pipeline_override = Some(yaml_recipe.post.qa_pipeline);
        run_quality_pipeline(&build_context, &environment, qa_pipeline_override).await?;
    }

    // Create build config (same as build command)
    let build_config = BuildConfig::default();

    // Generate recipe metadata (same as build command)
    let recipe_metadata = RecipeMetadata {
        name: yaml_recipe.metadata.name.clone(),
        version: yaml_recipe.metadata.version.clone(),
        description: yaml_recipe.metadata.description.clone().into(),
        homepage: yaml_recipe.metadata.homepage.clone(),
        license: Some(yaml_recipe.metadata.license.clone()),
        runtime_deps: yaml_recipe.metadata.dependencies.runtime.clone(),
        build_deps: yaml_recipe.metadata.dependencies.build.clone(),
    };

    // Generate SBOM and create manifest (EXACT same as build command)
    let (_sbom_files, manifest) = generate_sbom_and_manifest(
        &build_config,
        &build_context,
        &environment,
        recipe_metadata.runtime_deps.clone(),
        &recipe_metadata,
    )
    .await?;

    // Create and sign package (EXACT same as build command)
    let package_path =
        create_and_sign_package(&build_config, &build_context, &environment, manifest).await?;

    let duration = start.elapsed();

    ctx.tx
        .send(Event::OperationCompleted {
            operation: format!("Packed {package_name} v{package_version} from staging directory"),
            success: true,
        })
        .map_err(|_| OpsError::EventChannelClosed)?;

    // Create BuildReport
    Ok(BuildReport {
        package: package_name,
        version: package_version,
        output_path: package_path,
        duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        sbom_generated: true,
    })
}

/// Execute post-processing steps from recipe (same as build command)
async fn execute_post_steps(
    context: &BuildContext,
    environment: &mut BuildEnvironment,
    yaml_recipe: &YamlRecipe,
) -> Result<(), Error> {
    // Parse recipe into build plan to extract post steps
    let build_plan = BuildPlan::from_yaml(yaml_recipe, &context.recipe_path, None)?;

    if build_plan.post_steps.is_empty() {
        return Ok(());
    }

    context
        .event_sender
        .as_ref()
        .unwrap()
        .send(Event::OperationStarted {
            operation: "Executing post-processing steps".to_string(),
        })
        .map_err(|_| OpsError::EventChannelClosed)?;

    // Create working directory and security context
    let working_dir = environment.build_prefix().join("src");
    let mut initial_vars = HashMap::new();
    initial_vars.insert("NAME".to_string(), context.name.clone());
    initial_vars.insert("VERSION".to_string(), context.version.to_string());

    let mut security_context =
        SecurityContext::new(environment.build_prefix().to_path_buf(), initial_vars);
    security_context.set_current_dir(working_dir.clone());

    // Create builder API
    let resources = std::sync::Arc::new(sps2_resources::ResourceManager::default());
    let mut api = BuilderApi::new(working_dir, resources)?;

    // Execute each post step
    for step in &build_plan.post_steps {
        context
            .event_sender
            .as_ref()
            .unwrap()
            .send(Event::BuildStepStarted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            })
            .map_err(|_| OpsError::EventChannelClosed)?;

        execute_post_step_with_security(
            step,
            &mut api,
            environment,
            &mut security_context,
            None, // No sps2_config restriction for pack command
        )
        .await?;

        context
            .event_sender
            .as_ref()
            .unwrap()
            .send(Event::BuildStepCompleted {
                step: format!("{step:?}"),
                package: context.name.clone(),
            })
            .map_err(|_| OpsError::EventChannelClosed)?;
    }

    context
        .event_sender
        .as_ref()
        .unwrap()
        .send(Event::OperationCompleted {
            operation: "Post-processing steps completed".to_string(),
            success: true,
        })
        .map_err(|_| OpsError::EventChannelClosed)?;

    Ok(())
}

/// Validate recipe file exists and has correct extension
fn validate_recipe_file(recipe_path: &Path) -> Result<(), Error> {
    if !recipe_path.exists() {
        return Err(OpsError::RecipeNotFound {
            path: recipe_path.display().to_string(),
        }
        .into());
    }

    let extension = recipe_path.extension().and_then(|ext| ext.to_str());
    let is_valid = matches!(extension, Some("yaml" | "yml"));

    if !is_valid {
        return Err(OpsError::InvalidRecipe {
            path: recipe_path.display().to_string(),
            reason: "recipe must have .yaml or .yml extension".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validate staging directory exists and contains packageable content
fn validate_staging_directory(
    staging_dir: &Path,
    package_name: &str,
    package_version: &Version,
) -> Result<(), Error> {
    if !staging_dir.exists() {
        return Err(OpsError::StagingDirectoryNotFound {
            path: staging_dir.display().to_string(),
            package: format!("{package_name}:{package_version}"),
        }
        .into());
    }

    if !staging_dir.is_dir() {
        return Err(OpsError::InvalidStagingDirectory {
            path: staging_dir.display().to_string(),
            reason: "staging path is not a directory".to_string(),
        }
        .into());
    }

    // Check if directory has any content
    let entries =
        std::fs::read_dir(staging_dir).map_err(|e| OpsError::InvalidStagingDirectory {
            path: staging_dir.display().to_string(),
            reason: format!("cannot read staging directory: {e}"),
        })?;

    if entries.count() == 0 {
        return Err(OpsError::InvalidStagingDirectory {
            path: staging_dir.display().to_string(),
            reason: "staging directory is empty".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Determine output path for package
fn determine_output_path(
    output_dir: Option<&Path>,
    package_name: &str,
    package_version: &Version,
) -> PathBuf {
    let filename = format!("{package_name}-{package_version}-1.arm64.sp");

    output_dir.unwrap_or_else(|| Path::new(".")).join(filename)
}

/// Detect build systems used in build steps for QA pipeline routing
///
/// Maps build commands to build system names that the QA pipeline router understands.
/// This determines which QA pipeline profile to use (Rust, Python, C/C++, etc.).
fn detect_build_systems_from_steps(build_steps: &[BuildCommand]) -> HashSet<String> {
    let mut build_systems = HashSet::new();

    for step in build_steps {
        let build_system = match step {
            BuildCommand::Configure { .. } => "configure",
            BuildCommand::Make { .. } => "make",
            BuildCommand::Autotools { .. } => "autotools",
            BuildCommand::Cmake { .. } => "cmake",
            BuildCommand::Meson { .. } => "meson",
            BuildCommand::Cargo { .. } => "cargo",
            BuildCommand::Go { .. } => "go",
            BuildCommand::Python { .. } => "python",
            BuildCommand::NodeJs { .. } => "nodejs",
            BuildCommand::Command { .. } => "shell",
        };
        build_systems.insert(build_system.to_string());
    }

    build_systems
}
