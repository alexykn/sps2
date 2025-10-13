//! Build command implementation
//!
//! Handles package building from recipes.
//! Delegates to `sps2_builder` crate for the actual build logic.

use crate::{BuildReport, OpsCtx};
use sps2_builder::{parse_yaml_recipe, BuildContext};
use sps2_errors::{Error, OpsError};
use sps2_events::{AppEvent, BuildEvent, BuildSession, BuildTarget, EventEmitter, FailureContext};
use sps2_types::Version;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Build package from recipe (delegates to builder crate)
///
/// # Errors
///
/// Returns an error if:
/// - Recipe file doesn't exist or has invalid extension
/// - Recipe cannot be loaded or executed
/// - Build process fails
pub async fn build(
    ctx: &OpsCtx,
    recipe_path: &Path,
    output_dir: Option<&Path>,
    network: bool,
    jobs: Option<usize>,
) -> Result<BuildReport, Error> {
    let start = Instant::now();

    let correlation_label = format!("build:{}", recipe_path.display());
    let _correlation = ctx.push_correlation(correlation_label);

    ensure_recipe_path(recipe_path)?;
    let (package_name, package_version) = load_recipe_metadata(recipe_path).await?;
    let (session, target, session_id) =
        build_session(package_name.clone(), package_version.clone());

    ctx.emit(AppEvent::Build(BuildEvent::Started {
        session: session.clone(),
        target: target.clone(),
    }));

    let output_directory = resolve_output_directory(output_dir);
    let canonical_recipe_path = canonicalize_recipe_path(recipe_path)?;
    let build_context = BuildContext::new(
        package_name.clone(),
        package_version.clone(),
        canonical_recipe_path,
        output_directory,
    )
    .with_event_sender(ctx.tx.clone())
    .with_session_id(session_id.clone());

    let builder = configure_builder(ctx, network, jobs);

    // Use the builder with custom configuration
    let result = match builder.build(build_context).await {
        Ok(result) => result,
        Err(error) => {
            ctx.emit(AppEvent::Build(BuildEvent::Failed {
                session_id: session_id.clone(),
                target: target.clone(),
                failure: FailureContext::from_error(&error),
                phase: None,
                command: None,
            }));
            return Err(error);
        }
    };

    // Check if install was requested during recipe execution
    if result.install_requested {
        ctx.emit_operation_started("Building package");

        // Install the built package
        let package_path_str = result.package_path.to_string_lossy().to_string();
        let _install_report = crate::install(ctx, &[package_path_str], false).await?;

        ctx.emit_operation_completed(
            format!("Installed {package_name} {package_version} successfully"),
            true,
        );
    }

    let report = BuildReport {
        package: package_name,
        version: package_version,
        output_path: result.package_path,
        duration_ms: elapsed_millis(start),
        // SBOM soft-disabled: this will now be false
        sbom_generated: !result.sbom_files.is_empty(),
    };

    ctx.emit(AppEvent::Build(BuildEvent::Completed {
        session_id,
        target,
        artifacts: vec![report.output_path.clone()],
        duration_ms: report.duration_ms,
    }));

    Ok(report)
}

fn elapsed_millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn ensure_recipe_path(recipe_path: &Path) -> Result<(), Error> {
    if !recipe_path.exists() {
        return Err(OpsError::RecipeNotFound {
            path: recipe_path.display().to_string(),
        }
        .into());
    }

    let extension = recipe_path.extension().and_then(|ext| ext.to_str());
    if matches!(extension, Some("yaml" | "yml")) {
        return Ok(());
    }

    Err(OpsError::InvalidRecipe {
        path: recipe_path.display().to_string(),
        reason: "recipe must have .yaml or .yml extension".to_string(),
    }
    .into())
}

async fn load_recipe_metadata(recipe_path: &Path) -> Result<(String, Version), Error> {
    let yaml_recipe = parse_yaml_recipe(recipe_path).await?;
    let version = Version::parse(&yaml_recipe.metadata.version)?;
    Ok((yaml_recipe.metadata.name.clone(), version))
}

fn build_session(
    package_name: String,
    package_version: Version,
) -> (BuildSession, BuildTarget, String) {
    let session_id = uuid::Uuid::new_v4().to_string();
    let session = BuildSession {
        id: session_id.clone(),
        system: sps2_events::BuildSystem::Custom,
        cache_enabled: false,
    };
    let target = BuildTarget {
        package: package_name,
        version: package_version,
    };
    (session, target, session_id)
}

fn resolve_output_directory(output_dir: Option<&Path>) -> PathBuf {
    output_dir
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn canonicalize_recipe_path(recipe_path: &Path) -> Result<PathBuf, Error> {
    recipe_path.canonicalize().map_err(|e| {
        OpsError::InvalidRecipe {
            path: recipe_path.display().to_string(),
            reason: format!("failed to canonicalize recipe path: {e}"),
        }
        .into()
    })
}

fn configure_builder(ctx: &OpsCtx, network: bool, jobs: Option<usize>) -> sps2_builder::Builder {
    let mut builder_config = sps2_builder::BuildConfig::default();
    if network {
        builder_config.config.build.default_allow_network = true;
    }
    let _ = jobs;
    builder_config.sps2_config = Some(ctx.config.clone());

    sps2_builder::Builder::with_config(builder_config)
        .with_resolver(ctx.resolver.clone())
        .with_store(ctx.store.clone())
}
