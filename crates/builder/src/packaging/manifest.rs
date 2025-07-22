//! Package manifest and SBOM coordination

use crate::utils::events::send_event;
use crate::yaml::RecipeMetadata;
use crate::{BuildContext, BuildEnvironment, SbomFiles, SbomGenerator};
use sps2_errors::Error;
use sps2_events::AppEvent;
use sps2_manifest::Manifest;
use tokio::fs;

/// Generate SBOM and create package manifest
///
/// # Errors
///
/// Returns an error if:
/// - SBOM directory creation fails
/// - SBOM generation fails
/// - File system operations fail during SBOM creation
pub async fn generate_sbom_and_manifest(
    config: &crate::BuildConfig,
    context: &BuildContext,
    environment: &BuildEnvironment,
    runtime_deps: Vec<String>,
    recipe_metadata: &RecipeMetadata,
) -> Result<(SbomFiles, Manifest), Error> {
    // Generate SBOM
    send_event(
        context,
        Event::OperationStarted {
            operation: "Generating SBOM".to_string(),
        },
    );
    let sbom_files = generate_sbom(config, environment).await?;
    send_event(
        context,
        Event::OperationCompleted {
            operation: "SBOM generation completed".to_string(),
            success: true,
        },
    );

    // Create manifest
    send_event(
        context,
        Event::OperationStarted {
            operation: "Creating package manifest".to_string(),
        },
    );
    let manifest = create_manifest(
        context,
        runtime_deps,
        &sbom_files,
        recipe_metadata,
        environment,
    );
    send_event(
        context,
        Event::OperationCompleted {
            operation: "Package manifest created".to_string(),
            success: true,
        },
    );

    Ok((sbom_files, manifest))
}

/// Generate SBOM files
///
/// # Errors
///
/// Returns an error if:
/// - SBOM directory creation fails
/// - SBOM generator fails to scan staging directory
/// - File I/O operations fail during SBOM generation
pub async fn generate_sbom(
    config: &crate::BuildConfig,
    environment: &BuildEnvironment,
) -> Result<SbomFiles, Error> {
    let sbom_config = config.sbom_config().clone();

    // Set package name and version from build context

    let generator = SbomGenerator::new(
        sbom_config,
        environment.package_name().to_string(),
        environment.context.version.to_string(),
    );

    let staging_dir = environment.staging_dir();
    let sbom_dir = environment.build_prefix().join("sbom");
    fs::create_dir_all(&sbom_dir).await?;

    generator.generate_sbom(staging_dir, &sbom_dir).await
}

/// Create package manifest
pub fn create_manifest(
    context: &BuildContext,
    runtime_deps: Vec<String>,
    sbom_files: &SbomFiles,
    recipe_metadata: &RecipeMetadata,
    environment: &BuildEnvironment,
) -> Manifest {
    use sps2_manifest::{CompressionInfo, Dependencies, PackageInfo, SbomInfo};
    use sps2_types::format::CompressionFormatType;

    // Create SBOM info if files are available
    let sbom_info = sbom_files.spdx_hash.as_ref().map(|spdx_hash| SbomInfo {
        spdx: spdx_hash.clone(),
        cyclonedx: sbom_files.cyclonedx_hash.clone(),
    });

    // Create compression info
    let compression_info = Some(CompressionInfo {
        format: CompressionFormatType::Legacy,
        frame_size: None,
        frame_count: None,
    });

    // Generate Python metadata if this is a Python package
    let python_metadata = if environment.used_build_systems().contains("python") {
        Some(create_python_metadata_from_env(environment))
    } else {
        None
    };

    Manifest {
        format_version: sps2_types::PackageFormatVersion::CURRENT,
        package: PackageInfo {
            name: context.name.clone(),
            version: context.version.to_string(),
            revision: context.revision,
            arch: context.arch.clone(),
            description: recipe_metadata.description.clone(),
            homepage: recipe_metadata.homepage.clone(),
            license: recipe_metadata.license.clone(),
            compression: compression_info,
        },
        dependencies: Dependencies {
            runtime: runtime_deps,
            build: Vec::new(), // Build deps not included in final manifest
        },
        sbom: sbom_info,
        python: python_metadata,
    }
}

/// Create Python metadata for builder-centric approach
fn create_python_metadata_from_env(
    environment: &BuildEnvironment,
) -> sps2_types::PythonPackageMetadata {
    use std::collections::HashMap;

    // Extract metadata from build environment
    let requires_python = environment
        .get_extra_env("PYTHON_REQUIRES_VERSION")
        .unwrap_or_else(|| ">=3.8".to_string());

    let executables = environment
        .get_extra_env("PYTHON_ENTRY_POINTS")
        .and_then(|json_str| serde_json::from_str::<HashMap<String, String>>(&json_str).ok())
        .unwrap_or_default();

    // For builder-centric approach, wheel_file and requirements_file are not used
    // since the builder has already installed the package to staging
    sps2_types::PythonPackageMetadata {
        requires_python,
        wheel_file: String::new(), // Not used in builder-centric approach
        requirements_file: String::new(), // Not used in builder-centric approach
        executables,
    }
}
