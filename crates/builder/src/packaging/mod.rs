//! Packaging module for archive, compression, manifest, SBOM, and signing

pub mod archive;
pub mod compression;
pub mod manifest;
pub mod sbom;
pub mod signing;

use self::archive::create_deterministic_tar_archive;
use self::compression::compress_with_zstd;
use self::sbom::{SbomFiles, SbomGenerator};
use self::signing::PackageSigner;
use crate::utils::events::send_event;
use crate::utils::fileops::copy_directory_strip_live_prefix;
use crate::{BuildConfig, BuildContext, BuildEnvironment};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use sps2_manifest::Manifest;
use sps2_types::PythonPackageMetadata;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Create package archive and sign it
///
/// # Errors
///
/// Returns an error if:
/// - Package creation fails
/// - Package signing fails (when enabled)
pub async fn create_and_sign_package(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &BuildEnvironment,
    manifest: Manifest,
) -> Result<PathBuf, Error> {
    // Generate SBOM files first
    let sbom_files = generate_sbom_files(config, context, environment, &manifest).await?;

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

/// Generate SBOM files if enabled in the configuration.
async fn generate_sbom_files(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &BuildEnvironment,
    manifest: &Manifest,
) -> Result<SbomFiles, Error> {
    if !config.packaging_settings().sbom.enabled {
        return Ok(SbomFiles::default());
    }

    send_event(
        context,
        Event::OperationStarted {
            operation: "Generating SBOM".to_string(),
        },
    );

    let sbom_generator = SbomGenerator::new(
        config.packaging_settings().sbom.clone(),
        manifest.package.name.clone(),
        manifest.version().unwrap().to_string(),
    );

    let sbom_files = sbom_generator
        .generate_sbom(
            environment.staging_dir(),
            &config.build_settings().build_root,
        )
        .await?;

    send_event(
        context,
        Event::OperationCompleted {
            operation: "SBOM generation completed".to_string(),
            success: true,
        },
    );

    Ok(sbom_files)
}

/// Create the final package
///
/// # Errors
///
/// Returns an error if:
/// - Python package structure creation fails (for Python packages)
/// - Manifest serialization to TOML fails
/// - SP package archive creation fails
pub async fn create_package(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &BuildEnvironment,
    mut manifest: Manifest,
    sbom_files: SbomFiles,
) -> Result<PathBuf, Error> {
    let package_path = context.output_path();

    // Handle Python packages specially
    if environment.is_python_package() {
        let python_metadata = create_python_package_structure(environment, &manifest).await?;
        manifest.set_python_metadata(python_metadata);
    }

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
///
/// # Errors
///
/// Returns an error if:
/// - Directory creation fails
/// - File I/O operations fail (writing manifest, copying SBOM files)
/// - Tar archive creation fails or times out
/// - Zstd compression fails
/// - Cleanup operations fail
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

    // Copy staging directory contents, stripping the opt/pm/live prefix
    if staging_dir.exists() {
        copy_directory_strip_live_prefix(staging_dir, &package_temp_dir).await?;
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
    compress_with_zstd(
        &config.packaging_settings().compression,
        &tar_path,
        output_path,
    )
    .await?;
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
///
/// # Errors
///
/// Returns an error if:
/// - Package signing fails (when signing is enabled)
/// - Cryptographic operations fail during signing
pub async fn sign_package(
    config: &BuildConfig,
    context: &BuildContext,
    package_path: &Path,
) -> Result<(), Error> {
    if !config.packaging_settings().signing.enabled {
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

    let signer = PackageSigner::new(config.packaging_settings().signing.clone());

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

/// Create Python package structure and metadata
///
/// # Errors
///
/// Returns an error if:
/// - Required Python environment variables are missing
/// - Directory creation fails
/// - File copying operations fail (wheel, lockfile)
/// - Path operations fail
async fn create_python_package_structure(
    environment: &BuildEnvironment,
    manifest: &Manifest,
) -> Result<PythonPackageMetadata, Error> {
    // Get Python metadata from the build environment
    let wheel_path = environment
        .get_extra_env("PYTHON_WHEEL_PATH")
        .ok_or_else(|| BuildError::Failed {
            message: "Python wheel path not found in build environment".to_string(),
        })?;

    let lockfile_path = environment
        .get_extra_env("PYTHON_LOCKFILE_PATH")
        .ok_or_else(|| BuildError::Failed {
            message: "Python lockfile path not found in build environment".to_string(),
        })?;

    let entry_points_json = environment
        .get_extra_env("PYTHON_ENTRY_POINTS")
        .unwrap_or_else(|| "{}".to_string());

    let requires_python = environment
        .get_extra_env("PYTHON_REQUIRES_VERSION")
        .unwrap_or_else(|| ">=3.8".to_string());

    // Parse entry points
    let executables: HashMap<String, String> =
        serde_json::from_str(&entry_points_json).unwrap_or_default();

    // Create python/ subdirectory in staging
    let python_dir = environment
        .staging_dir()
        .join(manifest.filename().replace(".sp", ""))
        .join("python");
    fs::create_dir_all(&python_dir).await?;

    // Copy wheel file
    let wheel_src = PathBuf::from(&wheel_path);
    let wheel_filename = wheel_src.file_name().ok_or_else(|| BuildError::Failed {
        message: "Invalid wheel path".to_string(),
    })?;
    let wheel_dst = python_dir.join(wheel_filename);
    fs::copy(&wheel_src, &wheel_dst).await?;

    // Copy lockfile
    let lockfile_src = PathBuf::from(&lockfile_path);
    let lockfile_dst = python_dir.join("requirements.lock.txt");
    fs::copy(&lockfile_src, &lockfile_dst).await?;

    // Create Python metadata
    Ok(PythonPackageMetadata {
        requires_python,
        wheel_file: format!("python/{}", wheel_filename.to_string_lossy()),
        requirements_file: "python/requirements.lock.txt".to_string(),
        executables,
    })
}
