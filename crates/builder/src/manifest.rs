//! Package manifest and SBOM coordination

use crate::events::send_event;
use crate::yaml::RecipeMetadata;
use crate::{BuildContext, BuildEnvironment, SbomFiles, SbomGenerator};
use sps2_errors::Error;
use sps2_events::Event;
use sps2_manifest::Manifest;
use tokio::fs;

/// Generate SBOM and create package manifest
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
    let manifest = create_manifest(context, runtime_deps, &sbom_files, recipe_metadata);
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
pub async fn generate_sbom(
    config: &crate::BuildConfig,
    environment: &BuildEnvironment,
) -> Result<SbomFiles, Error> {
    let mut sbom_config = config.sbom_config.clone();

    // Set package name and version from build context
    sbom_config.package_name = Some(environment.package_name().to_string());
    sbom_config.package_version = Some(environment.context.version.to_string());

    let generator = SbomGenerator::new().with_config(sbom_config);

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
        python: None, // TODO: Add Python metadata support in builder
    }
}
