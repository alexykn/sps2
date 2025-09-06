//! Install command implementation
//!
//! Handles package installation with support for both local .sp files and remote packages.
//! Delegates to `sps2_install` crate for the actual installation logic.

use crate::{InstallReport, InstallRequest, OpsCtx};
use sps2_errors::{Error, OpsError};
use sps2_events::{AppEvent, EventEmitter, GeneralEvent, ProgressEvent, ResolverEvent};
use sps2_guard::{OperationResult as GuardOperationResult, PackageChange as GuardPackageChange};
use sps2_install::{InstallConfig, InstallContext, Installer};
use sps2_types::{PackageSpec, Version};
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

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

    // Check mode: preview what would be installed
    if ctx.check_mode {
        return preview_install(ctx, package_specs).await;
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
                ctx.emit_error("Failed to install remote packages");
                return Err(e);
            }
        }
    } else if remote_specs.is_empty() && !local_files.is_empty() {
        // All local files - use local installer
        match install_local_packages(ctx, &local_files).await {
            Ok(result) => result,
            Err(e) => {
                // Provide specific guidance for local file failures
                ctx.emit(AppEvent::General(GeneralEvent::error_with_details(
                    format!("Failed to install {} local packages", local_files.len()),
                    format!(
                        "Error: {e}. \n\nSuggested solutions:\n\
                        • Verify file paths are correct and files exist\n\
                        • Check file permissions (must be readable)\n\
                        • Ensure .sp files are not corrupted\n\
                        • Use absolute paths or './' prefix for current directory"
                    ),
                )));
                return Err(e);
            }
        }
    } else {
        // Mixed local and remote - use hybrid approach
        match install_mixed_packages(ctx, &remote_specs, &local_files).await {
            Ok(result) => result,
            Err(e) => {
                // Provide guidance for mixed installation failures
                ctx.emit(AppEvent::General(GeneralEvent::error_with_details(
                    format!(
                        "Failed to install mixed packages ({} remote, {} local)",
                        remote_specs.len(),
                        local_files.len()
                    ),
                    format!(
                        "Error: {e}. \n\nSuggested solutions:\n\
                        • Verify file paths are correct and files exist\n\
                        • Check file permissions (must be readable)\n\
                        • Ensure .sp files are not corrupted\n\
                        • Use absolute paths or './' prefix for current directory"
                    ),
                )));
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

/// Preview what would be installed without executing
#[allow(clippy::too_many_lines)]
async fn preview_install(ctx: &OpsCtx, package_specs: &[String]) -> Result<InstallReport, Error> {
    use std::collections::HashMap;

    // Parse install requests
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

    // Get currently installed packages to check for updates
    let installed_before = ctx.state.get_installed_packages().await?;
    let installed_map: HashMap<String, Version> = installed_before
        .iter()
        .map(|pkg| (pkg.name.clone(), pkg.version()))
        .collect();

    let mut preview_installed = Vec::new();
    let mut preview_updated = Vec::new();
    let mut new_packages_count = 0;
    let mut dependencies_added_count = 0;

    // Handle remote packages
    if !remote_specs.is_empty() {
        // Create resolution context
        let mut resolution_context = sps2_resolver::ResolutionContext::new();
        for spec in &remote_specs {
            resolution_context = resolution_context.add_runtime_dep(spec.clone());
        }

        // Resolve dependencies
        let resolution_result = ctx.resolver.resolve_with_sat(resolution_context).await?;

        // Process resolved packages
        for (package_id, node) in &resolution_result.nodes {
            let is_requested = remote_specs.iter().any(|spec| spec.name == package_id.name);
            let is_dependency = !is_requested;

            if let Some(existing_version) = installed_map.get(&package_id.name) {
                if existing_version != &package_id.version {
                    // Package would be updated
                    ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                        operation: "install".to_string(),
                        action: format!(
                            "Would update {} {} → {}",
                            package_id.name, existing_version, package_id.version
                        ),
                        details: HashMap::from([
                            ("current_version".to_string(), existing_version.to_string()),
                            ("new_version".to_string(), package_id.version.to_string()),
                            (
                                "source".to_string(),
                                match node.action {
                                    sps2_resolver::NodeAction::Download => "repository".to_string(),
                                    sps2_resolver::NodeAction::Local => "local file".to_string(),
                                },
                            ),
                        ]),
                    }));

                    preview_updated.push(crate::PackageChange {
                        name: package_id.name.clone(),
                        from_version: Some(existing_version.clone()),
                        to_version: Some(package_id.version.clone()),
                        size: None,
                    });
                }
            } else {
                // Package would be newly installed
                let action_text = if is_dependency {
                    format!("Would install {package_id} (dependency)")
                } else {
                    format!("Would install {package_id}")
                };

                ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
                    operation: "install".to_string(),
                    action: action_text,
                    details: HashMap::from([
                        ("version".to_string(), package_id.version.to_string()),
                        (
                            "source".to_string(),
                            match node.action {
                                sps2_resolver::NodeAction::Download => "repository".to_string(),
                                sps2_resolver::NodeAction::Local => "local file".to_string(),
                            },
                        ),
                        (
                            "type".to_string(),
                            if is_dependency {
                                "dependency".to_string()
                            } else {
                                "requested".to_string()
                            },
                        ),
                    ]),
                }));

                preview_installed.push(crate::PackageChange {
                    name: package_id.name.clone(),
                    from_version: None,
                    to_version: Some(package_id.version.clone()),
                    size: None,
                });

                if is_dependency {
                    dependencies_added_count += 1;
                } else {
                    new_packages_count += 1;
                }
            }
        }
    }

    // Handle local files
    for local_file in &local_files {
        // For local files, we can't easily resolve without reading the file
        // So we'll show a basic preview
        ctx.emit(AppEvent::General(GeneralEvent::CheckModePreview {
            operation: "install".to_string(),
            action: format!("Would install from local file: {}", local_file.display()),
            details: HashMap::from([
                ("source".to_string(), "local file".to_string()),
                ("path".to_string(), local_file.display().to_string()),
            ]),
        }));

        // Add to preview (we don't know the exact package name/version without reading)
        preview_installed.push(crate::PackageChange {
            name: format!(
                "local-{}",
                local_file.file_stem().unwrap_or_default().to_string_lossy()
            ),
            from_version: None,
            to_version: Some(Version::new(0, 0, 0)), // Placeholder
            size: None,
        });
        new_packages_count += 1;
    }

    // Emit summary
    let total_changes = preview_installed.len() + preview_updated.len();
    let mut categories = HashMap::new();
    categories.insert("new_packages".to_string(), new_packages_count);
    categories.insert("updated_packages".to_string(), preview_updated.len());
    if dependencies_added_count > 0 {
        categories.insert("dependencies_added".to_string(), dependencies_added_count);
    }

    ctx.emit(AppEvent::General(GeneralEvent::CheckModeSummary {
        operation: "install".to_string(),
        total_changes,
        categories,
    }));

    // Return preview report (no actual state changes)
    Ok(InstallReport {
        installed: preview_installed,
        updated: preview_updated,
        removed: Vec::new(),
        state_id: Uuid::nil(), // No state change in preview
        duration_ms: 0,
    })
}

/// Install remote packages using the high-performance parallel pipeline
#[allow(clippy::too_many_lines)] // Complex parallel pipeline orchestration with error handling
async fn install_remote_packages_parallel(
    ctx: &OpsCtx,
    specs: &[PackageSpec],
) -> Result<sps2_install::InstallResult, Error> {
    use sps2_events::{patterns::InstallProgressConfig, ProgressManager};
    // use sps2_state::PackageRef;
    use std::time::Instant;

    let start = Instant::now();

    // Create unified progress tracker using standardized patterns
    let progress_manager = ProgressManager::new();
    let install_config = InstallProgressConfig {
        operation_name: format!("Installing {} packages", specs.len()),
        package_count: specs.len() as u64,
        include_dependency_resolution: true,
    };

    let progress_id = progress_manager.create_install_tracker(install_config);

    // The new standardized progress tracker handles the initial event emission.

    // Phase 1: Dependency resolution
    ctx.emit(AppEvent::Resolver(ResolverEvent::ResolutionStarted {
        runtime_deps: specs.len(),
        build_deps: 0,
        local_files: 0,
        timeout_seconds: 300, // 5 minute timeout
    }));

    let mut resolution_context = sps2_resolver::ResolutionContext::new();
    for spec in specs {
        resolution_context = resolution_context.add_runtime_dep(spec.clone());
    }

    let resolution_result = match ctx.resolver.resolve_with_sat(resolution_context).await {
        Ok(result) => result,
        Err(e) => {
            // Emit helpful error event for resolution failures
            ctx.emit(AppEvent::General(GeneralEvent::error_with_details(
                "Dependency resolution failed".to_string(),
                format!("Error: {e}"),
            )));

            // Mark progress as failed
            ctx.emit(AppEvent::Progress(ProgressEvent::Failed {
                id: progress_id.clone(),
                error: format!("Pipeline execution failed: {e}"),
                completed_items: 0,
                partial_duration: std::time::Duration::from_secs(0),
            }));

            return Err(e);
        }
    };
    let execution_plan = resolution_result.execution_plan;
    let resolved_packages = resolution_result.nodes;

    progress_manager.update_phase_to_done(&progress_id, "Resolve", &ctx.tx);

    // Phase 2-4: Parallel execution (download, store, prepare)
    // Use the same approach as the regular installer with ParallelExecutor
    let exec_context = sps2_install::ExecutionContext::new()
        .with_event_sender(ctx.tx.clone())
        .with_security_policy(sps2_install::SecurityPolicy {
            verify_signatures: ctx.config.security.verify_signatures,
            allow_unsigned: ctx.config.security.allow_unsigned,
        });

    // Create parallel executor
    let resources = std::sync::Arc::new(sps2_resources::ResourceManager::default());
    let executor =
        sps2_install::ParallelExecutor::new(ctx.store.clone(), ctx.state.clone(), resources)?;

    // Execute parallel downloads and store packages
    let prepared_packages = match executor
        .execute_parallel(&execution_plan, &resolved_packages, &exec_context)
        .await
    {
        Ok(prepared_packages) => prepared_packages,
        Err(e) => {
            // Send helpful error context
            ctx.emit(AppEvent::General(GeneralEvent::error_with_details(
                "Installation failed during download/validation phase".to_string(),
                format!(
                    "Error: {e}. This may be due to network issues, package corruption, or insufficient disk space. \
                    Try running 'sps2 cleanup' to free space or check your network connection."
                ),
            )));

            // Mark progress as failed
            ctx.emit(AppEvent::Progress(ProgressEvent::Failed {
                id: progress_id.clone(),
                error: format!("Installation failed: {e}"),
                completed_items: 0,
                partial_duration: std::time::Duration::from_secs(0),
            }));

            return Err(e);
        }
    };

    progress_manager.update_phase_to_done(&progress_id, "Download", &ctx.tx);

    // Phase 5: Atomic installation
    ctx.emit_debug("DEBUG: Starting atomic installation");

    // Perform atomic installation using the prepared packages
    let mut atomic_installer =
        sps2_install::AtomicInstaller::new(ctx.state.clone(), ctx.store.clone()).await?;

    let install_context = sps2_install::InstallContext::new().with_event_sender(ctx.tx.clone());

    let install_result = atomic_installer
        .install(
            &install_context,
            &resolved_packages,
            Some(&prepared_packages),
        )
        .await?;

    ctx.emit_debug("DEBUG: Atomic installation completed");

    // Complete progress tracking
    progress_manager.complete_operation(&progress_id, &ctx.tx);

    // Send comprehensive completion metrics
    ctx.emit_debug(format!(
        "DEBUG: Installation metrics - Total: {}, Successful: {}, Duration: {:.2}s",
        specs.len(),
        install_result.installed_packages.len(),
        start.elapsed().as_secs_f64()
    ));

    // Return the install result from AtomicInstaller (already committed via 2PC)
    Ok(install_result)
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
            std::path::PathBuf::from(sps2_config::fixed_paths::LIVE_DIR),
            std::path::PathBuf::from(sps2_config::fixed_paths::BIN_DIR),
            std::path::PathBuf::from(format!("{}/lib", sps2_config::fixed_paths::LIVE_DIR)),
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
