//! Install command implementation
//!
//! Handles package installation with support for both local .sp files and remote packages.
//! Delegates to `sps2_install` crate for the actual installation logic.

use crate::{InstallReport, InstallRequest, OpsCtx};
use sps2_errors::{Error, InstallError, OpsError};
use sps2_events::Event;
use sps2_guard::{OperationResult as GuardOperationResult, PackageChange as GuardPackageChange};
use sps2_install::{InstallConfig, InstallContext, Installer, PipelineConfig, PipelineMaster};
use sps2_types::{PackageSpec, Version};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Install packages using the high-performance parallel pipeline
///
/// This function provides a unified installation workflow that seamlessly handles
/// both local .sp files and remote packages with optimal performance.
///
/// # Errors
///
/// Returns an error if:
/// - No packages are specified
/// - Package specifications cannot be parsed
/// - Installation fails
#[allow(clippy::too_many_lines)] // Complex orchestration function coordinating multiple subsystems
pub async fn install(ctx: &OpsCtx, package_specs: &[String]) -> Result<InstallReport, Error> {
    let start = Instant::now();

    if package_specs.is_empty() {
        return Err(OpsError::NoPackagesSpecified.into());
    }

    // Event emission moved to operations layer where actual work happens

    // Parse install requests and separate local files from remote packages
    let install_requests = parse_install_requests(package_specs)?;
    let mut remote_specs = Vec::new();
    let mut local_files = Vec::new();

    for request in install_requests {
        match request {
            InstallRequest::Remote(spec) => {
                remote_specs.push(spec);
            }
            InstallRequest::LocalFile(path) => {
                local_files.push(path);
            }
        }
    }

    // Get currently installed packages before install to track changes
    let installed_before = ctx.state.get_installed_packages().await?;
    let installed_map: std::collections::HashMap<String, Version> = installed_before
        .iter()
        .map(|pkg| (pkg.name.clone(), pkg.version()))
        .collect();

    // Use different strategies based on the mix of packages with enhanced error handling
    let result = if !remote_specs.is_empty() && local_files.is_empty() {
        // All remote packages - use high-performance parallel pipeline
        match install_remote_packages_parallel(ctx, &remote_specs).await {
            Ok(result) => result,
            Err(e) => {
                // Provide specific guidance for remote package failures
                ctx.tx
                    .send(Event::Error {
                        message: format!(
                            "Failed to install {} remote packages",
                            remote_specs.len()
                        ),
                        details: Some(format!(
                            "Error: {e}. \n\nSuggested solutions:\n\
                            • Check your internet connection\n\
                            • Run 'sps2 reposync' to update package index\n\
                            • Verify package names with 'sps2 search <package>'\n\
                            • For version constraints, use format: package>=1.0.0"
                        )),
                    })
                    .ok();
                return Err(e);
            }
        }
    } else if remote_specs.is_empty() && !local_files.is_empty() {
        // All local files - use local installer
        match install_local_packages(ctx, &local_files).await {
            Ok(result) => result,
            Err(e) => {
                // Provide specific guidance for local file failures
                ctx.tx
                    .send(Event::Error {
                        message: format!("Failed to install {} local packages", local_files.len()),
                        details: Some(format!(
                            "Error: {e}. \n\nSuggested solutions:\n\
                            • Verify file paths are correct and files exist\n\
                            • Check file permissions (must be readable)\n\
                            • Ensure .sp files are not corrupted\n\
                            • Use absolute paths or './' prefix for current directory"
                        )),
                    })
                    .ok();
                return Err(e);
            }
        }
    } else {
        // Mixed local and remote - use hybrid approach
        match install_mixed_packages(ctx, &remote_specs, &local_files).await {
            Ok(result) => result,
            Err(e) => {
                // Provide guidance for mixed installation failures
                ctx.tx
                    .send(Event::Error {
                        message: format!(
                            "Failed to install mixed packages ({} remote, {} local)",
                            remote_specs.len(),
                            local_files.len()
                        ),
                        details: Some(format!(
                            "Error: {e}. \n\nSuggested solutions:\n\
                            • Try installing remote and local packages separately\n\
                            • Check both network connectivity and local file access\n\
                            • Run with --debug flag for detailed error information\n\
                            • Consider using 'sps2 install package1 package2' for remote-only"
                        )),
                    })
                    .ok();
                return Err(e);
            }
        }
    };

    // Convert to report format with proper change tracking
    let report = InstallReport {
        installed: result
            .installed_packages
            .iter()
            .map(|pkg| {
                crate::PackageChange {
                    name: pkg.name.clone(),
                    from_version: None,
                    to_version: Some(pkg.version.clone()),
                    size: None, // TODO: Get size from store when available
                }
            })
            .collect(),
        updated: result
            .updated_packages
            .iter()
            .map(|pkg| crate::PackageChange {
                name: pkg.name.clone(),
                from_version: installed_map.get(&pkg.name).cloned(),
                to_version: Some(pkg.version.clone()),
                size: None,
            })
            .collect(),
        removed: result
            .removed_packages
            .iter()
            .map(|pkg| crate::PackageChange {
                name: pkg.name.clone(),
                from_version: Some(pkg.version.clone()),
                to_version: None,
                size: None,
            })
            .collect(),
        state_id: result.state_id,
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
    };

    // Event emission moved to operations layer where actual work happens

    Ok(report)
}

/// Install remote packages using the high-performance parallel pipeline
#[allow(clippy::too_many_lines)] // Complex parallel pipeline orchestration with error handling
async fn install_remote_packages_parallel(
    ctx: &OpsCtx,
    specs: &[PackageSpec],
) -> Result<sps2_install::InstallResult, Error> {
    use sps2_events::{ProgressManager, ProgressPhase};
    use sps2_state::PackageRef;

    // Create unified progress tracker for the entire install operation
    let progress_manager = ProgressManager::new();
    let install_phases = vec![
        ProgressPhase::new("resolve", "Resolving dependencies").with_weight(0.1),
        ProgressPhase::new("download", "Downloading packages").with_weight(0.5),
        ProgressPhase::new("validate", "Validating packages").with_weight(0.15),
        ProgressPhase::new("stage", "Staging packages").with_weight(0.15),
        ProgressPhase::new("commit", "Committing state").with_weight(0.1),
    ];

    let progress_id = progress_manager.start_operation(
        "install",
        "Installing packages",
        Some(specs.len() as u64),
        install_phases,
        ctx.tx.clone(),
    );

    // Send overall progress start event
    ctx.tx
        .send(Event::ProgressStarted {
            id: progress_id.clone(),
            operation: format!("Installing {} packages", specs.len()),
            total: Some(specs.len() as u64),
            phases: vec![
                ProgressPhase::new("resolve", "Resolving dependencies").with_weight(0.1),
                ProgressPhase::new("download", "Downloading packages").with_weight(0.5),
                ProgressPhase::new("validate", "Validating packages").with_weight(0.15),
                ProgressPhase::new("stage", "Staging packages").with_weight(0.15),
                ProgressPhase::new("commit", "Committing state").with_weight(0.1),
            ],
        })
        .ok();

    // Phase 1: Dependency resolution
    ctx.tx
        .send(Event::ResolvingDependencies {
            package: "batch".to_string(),
        })
        .ok();

    let mut resolution_context = sps2_resolver::ResolutionContext::new();
    for spec in specs {
        resolution_context = resolution_context.add_runtime_dep(spec.clone());
    }

    let resolution_result = match ctx
        .resolver
        .resolve_with_sat(resolution_context, Some(&ctx.tx))
        .await
    {
        Ok(result) => result,
        Err(e) => {
            // Emit helpful error event for resolution failures
            ctx.tx
                .send(Event::Error {
                    message: "Package resolution failed".to_string(),
                    details: Some(format!(
                        "Error: {e}. \n\nPossible reasons:\n\
                        • Package name or version typo.\n\
                        • Package not available in the current repositories.\n\
                        • Version constraints are unsatisfiable.\n\
                        \nSuggested solutions:\n\
                        • Double-check package name and version specs.\n\
                        • Run 'sps2 search <package_name>' to find available packages.\n\
                        • Run 'sps2 reposync' to update your package index."
                    )),
                })
                .ok();

            // Mark progress as failed
            ctx.tx
                .send(Event::ProgressFailed {
                    id: progress_id.clone(),
                    error: e.to_string(),
                })
                .ok();

            return Err(e);
        }
    };
    let execution_plan = resolution_result.execution_plan;
    let resolved_packages = resolution_result.nodes;

    // Update progress - resolution complete
    progress_manager.update_progress(&progress_id, 1, Some(specs.len() as u64), &ctx.tx);
    progress_manager.change_phase(&progress_id, 1, &ctx.tx);

    // Phase 2-4: Pipeline execution (download, validate, stage)
    let pipeline_config = PipelineConfig {
        max_downloads: 4,                // Conservative default
        max_decompressions: 2,           // CPU intensive
        max_validations: 3,              // I/O and compute
        enable_streaming: true,          // Enable streaming optimization
        buffer_size: 256 * 1024,         // 256KB buffers
        memory_limit: 100 * 1024 * 1024, // 100MB memory limit
        ..PipelineConfig::default()
    };

    // Derive staging base path from StateManager for test isolation
    let staging_base_path = ctx.state.state_path().join("staging");
    let pipeline =
        PipelineMaster::new(pipeline_config, ctx.store.clone(), staging_base_path).await?;

    // Execute pipeline with comprehensive error handling
    let batch_result = match pipeline
        .execute_batch(&execution_plan, &resolved_packages, &ctx.tx)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            // Send helpful error context
            ctx.tx
                .send(Event::Error {
                    message: "Installation failed during download/validation phase".to_string(),
                    details: Some(format!(
                        "Error: {e}. This may be due to network issues, package corruption, or insufficient disk space. \
                        Try running 'sps2 cleanup' to free space or check your network connection."
                    )),
                })
                .ok();

            // Mark progress as failed
            ctx.tx
                .send(Event::ProgressFailed {
                    id: progress_id,
                    error: e.to_string(),
                })
                .ok();

            return Err(e);
        }
    };

    // Phase 5: State management integration
    progress_manager.change_phase(&progress_id, 4, &ctx.tx);

    ctx.tx
        .send(Event::DebugLog {
            message: "DEBUG: Starting state management integration".to_string(),
            context: std::collections::HashMap::from([
                (
                    "successful_packages".to_string(),
                    batch_result.successful_packages.len().to_string(),
                ),
                (
                    "failed_packages".to_string(),
                    batch_result.failed_packages.len().to_string(),
                ),
            ]),
        })
        .ok();

    // Begin state transition
    let transition = ctx.state.begin_transition("install packages").await?;
    let new_state_id = transition.to;

    // Create package references for all successfully installed packages
    let mut packages_added = Vec::new();
    for package_id in &batch_result.successful_packages {
        // Get the actual hash from the batch result
        let hash = batch_result.package_hashes.get(package_id).ok_or_else(|| {
            InstallError::AtomicOperationFailed {
                message: format!("Missing hash for package {}", package_id.name),
            }
        })?;

        let package_ref = PackageRef {
            state_id: new_state_id,
            package_id: package_id.clone(),
            hash: hash.to_hex(),
            size: 1024 * 1024, // TODO: Get actual size from store
        };
        packages_added.push(package_ref);
    }

    ctx.tx
        .send(Event::DebugLog {
            message: format!(
                "DEBUG: Committing state transition with {} packages",
                packages_added.len()
            ),
            context: std::collections::HashMap::from([
                ("new_state_id".to_string(), new_state_id.to_string()),
                (
                    "packages_count".to_string(),
                    packages_added.len().to_string(),
                ),
            ]),
        })
        .ok();

    // Commit the state transition with the installed packages
    ctx.state
        .commit_transition(
            transition,
            packages_added,
            Vec::new(), // No packages removed
        )
        .await?;

    ctx.tx
        .send(Event::DebugLog {
            message: "DEBUG: State transition committed successfully".to_string(),
            context: std::collections::HashMap::new(),
        })
        .ok();

    // Complete progress tracking
    progress_manager.complete_operation(&progress_id, &ctx.tx);

    // Send comprehensive completion metrics
    ctx.tx
        .send(Event::DebugLog {
            message: format!(
                "Install completed: {} packages, {:.1} MB/s avg speed, {:.1}% efficiency",
                batch_result.stats.total_packages,
                batch_result.stats.avg_download_speed / (1024.0 * 1024.0),
                batch_result.stats.concurrency_efficiency * 100.0
            ),
            context: std::collections::HashMap::from([
                (
                    "total_downloaded".to_string(),
                    batch_result.stats.total_downloaded.to_string(),
                ),
                (
                    "peak_memory".to_string(),
                    batch_result.peak_memory_usage.to_string(),
                ),
                (
                    "duration_ms".to_string(),
                    batch_result.duration.as_millis().to_string(),
                ),
                (
                    "rollback_performed".to_string(),
                    batch_result.rollback_performed.to_string(),
                ),
            ]),
        })
        .ok();

    // Convert batch result to install result with actual state ID
    Ok(sps2_install::InstallResult {
        installed_packages: batch_result.successful_packages,
        updated_packages: Vec::new(), // Pipeline doesn't track updates separately
        removed_packages: Vec::new(), // No packages removed during install
        state_id: new_state_id,
    })
}

/// Install local packages using the regular installer
async fn install_local_packages(
    ctx: &OpsCtx,
    files: &[PathBuf],
) -> Result<sps2_install::InstallResult, Error> {
    // Create installer for local files
    let config = InstallConfig::default();
    let mut installer = Installer::new(
        config,
        ctx.resolver.clone(),
        ctx.state.clone(),
        ctx.store.clone(),
    );

    // Build install context for local files
    let install_context = InstallContext::new()
        .with_event_sender(ctx.tx.clone())
        .with_local_files(files.to_vec());

    // Execute installation
    installer.install(install_context).await
}

/// Install mixed local and remote packages using hybrid approach
async fn install_mixed_packages(
    ctx: &OpsCtx,
    remote_specs: &[PackageSpec],
    local_files: &[PathBuf],
) -> Result<sps2_install::InstallResult, Error> {
    // For mixed installs, use the regular installer for now
    // TODO: Optimize this by using pipeline for remote and merging results
    let config = InstallConfig::default();
    let mut installer = Installer::new(
        config,
        ctx.resolver.clone(),
        ctx.state.clone(),
        ctx.store.clone(),
    );

    // Build install context with both remote and local
    let mut install_context = InstallContext::new()
        .with_event_sender(ctx.tx.clone())
        .with_local_files(local_files.to_vec());

    for spec in remote_specs {
        install_context = install_context.add_package(spec.clone());
    }

    // Execute installation
    installer.install(install_context).await
}

/// Parse install requests from string specifications
fn parse_install_requests(specs: &[String]) -> Result<Vec<InstallRequest>, Error> {
    let mut requests = Vec::new();

    for spec in specs {
        if Path::new(spec)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sp"))
            && Path::new(spec).exists()
        {
            // Local file
            requests.push(InstallRequest::LocalFile(PathBuf::from(spec)));
        } else {
            // Remote package with version constraints
            let package_spec = PackageSpec::parse(spec)?;
            requests.push(InstallRequest::Remote(package_spec));
        }
    }

    Ok(requests)
}

/// Convert `InstallReport` to `GuardOperationResult` for guard integration
fn create_guard_operation_result(report: &InstallReport) -> GuardOperationResult {
    GuardOperationResult {
        installed: report
            .installed
            .iter()
            .map(|pkg| GuardPackageChange {
                name: pkg.name.clone(),
                from_version: pkg
                    .from_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                to_version: pkg
                    .to_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                size: pkg.size,
            })
            .collect(),
        updated: report
            .updated
            .iter()
            .map(|pkg| GuardPackageChange {
                name: pkg.name.clone(),
                from_version: pkg
                    .from_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                to_version: pkg
                    .to_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                size: pkg.size,
            })
            .collect(),
        removed: report
            .removed
            .iter()
            .map(|pkg| GuardPackageChange {
                name: pkg.name.clone(),
                from_version: pkg
                    .from_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                to_version: pkg
                    .to_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
                size: pkg.size,
            })
            .collect(),
        state_id: report.state_id,
        duration_ms: report.duration_ms,
        modified_directories: vec![
            std::path::PathBuf::from("/opt/pm/live"),
            std::path::PathBuf::from("/opt/pm/live/bin"),
            std::path::PathBuf::from("/opt/pm/live/lib"),
        ],
        install_triggered: false, // Standard install operation
    }
}

/// Install packages with state verification enabled
///
/// This wrapper uses the advanced `GuardedOperation` pattern providing:
/// - Cache warming before operation
/// - Operation-specific verification scoping
/// - Progressive verification when appropriate
/// - Smart cache invalidation after operation
///
/// # Errors
///
/// Returns an error if:
/// - Pre-install verification fails (when `fail_on_discrepancy` is true)
/// - Installation fails
/// - Post-install verification fails (when `fail_on_discrepancy` is true)
pub async fn install_with_verification(
    ctx: &OpsCtx,
    package_specs: &[String],
) -> Result<InstallReport, Error> {
    let package_specs_vec = package_specs.iter().map(ToString::to_string).collect();

    ctx.guarded_install(package_specs_vec)
        .execute(|| async {
            let report = install(ctx, package_specs).await?;
            let guard_result = create_guard_operation_result(&report);
            Ok((report, guard_result))
        })
        .await
}
