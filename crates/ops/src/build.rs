//! Build command implementation
//!
//! Handles package building from recipes.
//! Delegates to `sps2_builder` crate for the actual build logic.

use crate::{BuildReport, OpsCtx};
use sps2_builder::{parse_yaml_recipe, BuildContext};
use sps2_errors::{Error, OpsError};
use sps2_events::Event;
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

    ctx.tx
        .send(Event::BuildStarting {
            package: "unknown".to_string(), // Will be determined from recipe
            version: Version::parse("0.0.0").unwrap_or_else(|_| Version::new(0, 0, 0)),
        })
        .ok();

    // Load and execute recipe to get package metadata
    // We already validated that extension is yaml or yml
    let yaml_recipe = parse_yaml_recipe(recipe_path).await?;
    let package_name = yaml_recipe.metadata.name.clone();
    let package_version = Version::parse(&yaml_recipe.metadata.version)?;

    // Send updated build starting event with correct info
    ctx.tx
        .send(Event::BuildStarting {
            package: package_name.clone(),
            version: package_version.clone(),
        })
        .ok();

    // Create build context
    let output_directory = output_dir.map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        PathBuf::from,
    );

    // Canonicalize recipe path to ensure it's absolute
    let canonical_recipe_path =
        recipe_path
            .canonicalize()
            .map_err(|e| OpsError::InvalidRecipe {
                path: recipe_path.display().to_string(),
                reason: format!("failed to canonicalize recipe path: {e}"),
            })?;

    let build_context = BuildContext::new(
        package_name.clone(),
        package_version.clone(),
        canonical_recipe_path,
        output_directory,
    )
    .with_event_sender(ctx.tx.clone());

    // Configure builder with network and jobs options
    let mut builder_config = sps2_builder::BuildConfig::default();
    if network {
        builder_config.allow_network = true;
    }
    if let Some(job_count) = jobs {
        builder_config.build_jobs = Some(job_count);
    }
    // Pass the sps2 config for command validation
    builder_config.sps2_config = Some(ctx.config.clone());

    // Create builder with custom configuration
    let builder = sps2_builder::Builder::with_config(builder_config)
        .with_resolver(ctx.resolver.clone())
        .with_store(ctx.store.clone());

    // Use the builder with custom configuration
    let result = builder.build(build_context).await?;

    // Check if install was requested during recipe execution
    if result.install_requested {
        ctx.tx
            .send(Event::OperationStarted {
                operation: format!(
                    "Installing {package_name} {package_version} (requested by recipe)"
                ),
            })
            .ok();

        // Install the built package
        let package_path_str = result.package_path.to_string_lossy().to_string();
        let _install_report = crate::install(ctx, &[package_path_str]).await?;

        ctx.tx
            .send(Event::OperationCompleted {
                operation: format!("Installed {package_name} {package_version} successfully"),
                success: true,
            })
            .ok();
    }

    let report = BuildReport {
        package: package_name,
        version: package_version,
        output_path: result.package_path,
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        sbom_generated: !result.sbom_files.is_empty(),
    };

    ctx.tx
        .send(Event::BuildCompleted {
            package: report.package.clone(),
            version: report.version.clone(),
            path: report.output_path.clone(),
        })
        .ok();

    Ok(report)
}
