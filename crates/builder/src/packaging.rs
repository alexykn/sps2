//! Package creation and signing functionality

use crate::archive::create_deterministic_tar_archive;
use crate::compression::compress_with_zstd;
use crate::events::send_event;
use crate::fileops::copy_directory_recursive;
use crate::{BuildConfig, BuildContext, BuildEnvironment, PackageSigner, SbomFiles};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use sps2_manifest::Manifest;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Create package archive and sign it
pub async fn create_and_sign_package(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &BuildEnvironment,
    manifest: Manifest,
    sbom_files: SbomFiles,
) -> Result<PathBuf, Error> {
    // Package the result
    send_event(
        context,
        Event::OperationStarted {
            operation: "Creating package archive".to_string(),
        },
    );
    let package_path = create_package(config, context, environment, manifest, sbom_files).await?;
    send_event(
        context,
        Event::OperationCompleted {
            operation: format!("Package created: {}", package_path.display()),
            success: true,
        },
    );

    // Sign the package if configured
    sign_package(config, context, &package_path).await?;

    Ok(package_path)
}

/// Create the final package
pub async fn create_package(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &BuildEnvironment,
    manifest: Manifest,
    sbom_files: SbomFiles,
) -> Result<PathBuf, Error> {
    let package_path = context.output_path();

    // Create package using the real manifest data
    let manifest_string = toml::to_string(&manifest).map_err(|e| BuildError::Failed {
        message: format!("failed to serialize manifest: {e}"),
    })?;

    // Create proper .sp archive with manifest and SBOM files
    create_sp_package(
        config,
        context,
        environment.staging_dir(),
        &package_path,
        &manifest_string,
        &sbom_files,
    )
    .await?;

    Ok(package_path)
}

/// Create a .sp package archive with manifest, SBOM files, and tar+zstd compression
pub async fn create_sp_package(
    config: &BuildConfig,
    context: &BuildContext,
    staging_dir: &Path,
    output_path: &Path,
    manifest_content: &str,
    sbom_files: &SbomFiles,
) -> Result<(), Error> {
    // Create the directory structure for .sp package
    let package_dir = staging_dir.parent().ok_or_else(|| BuildError::Failed {
        message: "Invalid staging directory path".to_string(),
    })?;

    let package_temp_dir = package_dir.join("package_temp");
    fs::create_dir_all(&package_temp_dir).await?;

    // Step 1: Create manifest.toml in package root
    let manifest_path = package_temp_dir.join("manifest.toml");
    fs::write(&manifest_path, manifest_content).await?;

    // Step 2: Copy SBOM files
    if let Some(spdx_path) = &sbom_files.spdx_path {
        let dst_path = package_temp_dir.join("sbom.spdx.json");
        fs::copy(spdx_path, &dst_path).await?;
    }

    if let Some(cdx_path) = &sbom_files.cyclonedx_path {
        let dst_path = package_temp_dir.join("sbom.cdx.json");
        fs::copy(cdx_path, &dst_path).await?;
    }

    // Step 3: Copy staging directory contents as package files
    send_event(
        context,
        Event::OperationStarted {
            operation: "Copying package files".to_string(),
        },
    );
    let files_dir = package_temp_dir.join("files");

    // The staging directory contains the full /opt/pm/live structure
    // We need to extract only the final installation contents to make them relative
    let live_dir = staging_dir.join("opt").join("pm").join("live");
    if live_dir.exists() {
        copy_directory_recursive(&live_dir, &files_dir).await?;
    } else if staging_dir.exists() {
        // Fallback: copy staging dir directly if /opt/pm/live structure doesn't exist
        copy_directory_recursive(staging_dir, &files_dir).await?;
    } else {
        // Create empty files directory if staging doesn't exist
        fs::create_dir_all(&files_dir).await?;
    }
    send_event(
        context,
        Event::OperationCompleted {
            operation: "Package files copied".to_string(),
            success: true,
        },
    );

    // Step 4: Create deterministic tar archive
    send_event(
        context,
        Event::OperationStarted {
            operation: "Creating tar archive".to_string(),
        },
    );

    // Debug: List contents before tar creation
    send_event(
        context,
        Event::DebugLog {
            message: format!("Creating tar from: {}", package_temp_dir.display()),
            context: std::collections::HashMap::new(),
        },
    );

    let tar_path = package_temp_dir.join("package.tar");

    // Add timeout for tar creation to prevent hanging
    let tar_result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        create_deterministic_tar_archive(&package_temp_dir, &tar_path),
    )
    .await;

    match tar_result {
        Ok(result) => result?,
        Err(_) => {
            return Err(BuildError::Failed {
                message: "Tar archive creation timed out after 30 seconds".to_string(),
            }
            .into());
        }
    }

    send_event(
        context,
        Event::OperationCompleted {
            operation: "Tar archive created".to_string(),
            success: true,
        },
    );

    // Step 5: Compress with zstd at configured level
    send_event(
        context,
        Event::OperationStarted {
            operation: "Compressing package with zstd".to_string(),
        },
    );
    compress_with_zstd(&config.compression_config, &tar_path, output_path).await?;
    send_event(
        context,
        Event::OperationCompleted {
            operation: "Package compression completed".to_string(),
            success: true,
        },
    );

    // Step 6: Cleanup temporary files
    fs::remove_dir_all(&package_temp_dir).await?;

    Ok(())
}

/// Sign the package if signing is enabled
pub async fn sign_package(
    config: &BuildConfig,
    context: &BuildContext,
    package_path: &Path,
) -> Result<(), Error> {
    if !config.signing_config.enabled {
        return Ok(());
    }

    send_event(
        context,
        Event::OperationStarted {
            operation: format!(
                "Signing package {}",
                package_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ),
        },
    );

    let signer = PackageSigner::new(config.signing_config.clone());

    match signer.sign_package(package_path).await? {
        Some(sig_path) => {
            send_event(
                context,
                Event::OperationCompleted {
                    operation: format!("Package signed: {}", sig_path.display()),
                    success: true,
                },
            );
        }
        None => {
            // Signing was disabled
            send_event(
                context,
                Event::OperationCompleted {
                    operation: "Package signing skipped (disabled)".to_string(),
                    success: true,
                },
            );
        }
    }

    Ok(())
}
