//! Large operations that delegate to specialized crates

use crate::{BuildReport, ChangeType, InstallReport, InstallRequest, OpChange, OpReport, OpsCtx};
use spsv2_builder::{BuildConfig, BuildContext};
use spsv2_errors::{Error, OpsError};
use spsv2_events::Event;
use spsv2_install::{InstallConfig, InstallContext, Installer, UninstallContext, UpdateContext};
use spsv2_types::{PackageSpec, Version};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Install packages (delegates to install crate)
pub async fn install(ctx: &OpsCtx, package_specs: &[String]) -> Result<InstallReport, Error> {
    let start = Instant::now();

    if package_specs.is_empty() {
        return Err(OpsError::NoPackagesSpecified.into());
    }

    // Parse install requests
    let install_requests = parse_install_requests(package_specs)?;

    ctx.tx.send(Event::InstallStarting {
        packages: package_specs.iter().map(ToString::to_string).collect(),
    });

    // Create installer with configuration
    let config = InstallConfig::default();
    let mut installer = Installer::new(
        config,
        ctx.resolver.clone(),
        ctx.state.clone(),
        ctx.store.clone(),
    );

    // Build install context
    let mut install_context = InstallContext::new().with_event_sender(ctx.tx.clone());

    for request in install_requests {
        match request {
            InstallRequest::Remote(spec) => {
                install_context = install_context.add_package(spec);
            }
            InstallRequest::LocalFile(path) => {
                install_context = install_context.add_local_file(path);
            }
        }
    }

    // Execute installation
    let result = installer.install(install_context).await?;

    // Convert to report format
    let report = InstallReport {
        installed: result
            .installed_packages
            .iter()
            .map(|pkg| {
                crate::types::PackageChange {
                    name: pkg.name.clone(),
                    from_version: None,
                    to_version: Some(pkg.version.clone()),
                    size: None, // TODO: Get size from store
                }
            })
            .collect(),
        updated: result
            .updated_packages
            .iter()
            .map(|pkg| {
                crate::types::PackageChange {
                    name: pkg.name.clone(),
                    from_version: None, // TODO: Get previous version
                    to_version: Some(pkg.version.clone()),
                    size: None,
                }
            })
            .collect(),
        removed: result
            .removed_packages
            .iter()
            .map(|pkg| crate::types::PackageChange {
                name: pkg.name.clone(),
                from_version: Some(pkg.version.clone()),
                to_version: None,
                size: None,
            })
            .collect(),
        state_id: result.state_id,
        duration_ms: start.elapsed().as_millis() as u64,
    };

    ctx.tx.send(Event::InstallCompleted {
        packages: result
            .installed_packages
            .iter()
            .map(|pkg| pkg.name.clone())
            .collect(),
        state_id: result.state_id,
    });

    Ok(report)
}

/// Update packages (delegates to install crate)
pub async fn update(ctx: &OpsCtx, package_names: &[String]) -> Result<InstallReport, Error> {
    let start = Instant::now();

    ctx.tx.send(Event::UpdateStarting {
        packages: if package_names.is_empty() {
            vec!["all".to_string()]
        } else {
            package_names.to_vec()
        },
    });

    // Create installer
    let config = InstallConfig::default();
    let mut installer = Installer::new(
        config,
        ctx.resolver.clone(),
        ctx.state.clone(),
        ctx.store.clone(),
    );

    // Build update context
    let mut update_context = UpdateContext::new()
        .with_upgrade(false) // Update mode (respect upper bounds)
        .with_event_sender(ctx.tx.clone());

    for package_name in package_names {
        update_context = update_context.add_package(package_name.clone());
    }

    // Execute update
    let result = installer.update(update_context).await?;

    // Convert to report format
    let report = InstallReport {
        installed: result
            .installed_packages
            .iter()
            .map(|pkg| crate::types::PackageChange {
                name: pkg.name.clone(),
                from_version: None,
                to_version: Some(pkg.version.clone()),
                size: None,
            })
            .collect(),
        updated: result
            .updated_packages
            .iter()
            .map(|pkg| {
                crate::types::PackageChange {
                    name: pkg.name.clone(),
                    from_version: None, // TODO: Get previous version
                    to_version: Some(pkg.version.clone()),
                    size: None,
                }
            })
            .collect(),
        removed: result
            .removed_packages
            .iter()
            .map(|pkg| crate::types::PackageChange {
                name: pkg.name.clone(),
                from_version: Some(pkg.version.clone()),
                to_version: None,
                size: None,
            })
            .collect(),
        state_id: result.state_id,
        duration_ms: start.elapsed().as_millis() as u64,
    };

    ctx.tx.send(Event::UpdateCompleted {
        packages: result
            .updated_packages
            .iter()
            .map(|pkg| pkg.name.clone())
            .collect(),
        state_id: result.state_id,
    });

    Ok(report)
}

/// Upgrade packages (delegates to install crate)
pub async fn upgrade(ctx: &OpsCtx, package_names: &[String]) -> Result<InstallReport, Error> {
    let start = Instant::now();

    ctx.tx.send(Event::UpgradeStarting {
        packages: if package_names.is_empty() {
            vec!["all".to_string()]
        } else {
            package_names.to_vec()
        },
    });

    // Create installer
    let config = InstallConfig::default();
    let mut installer = Installer::new(
        config,
        ctx.resolver.clone(),
        ctx.state.clone(),
        ctx.store.clone(),
    );

    // Build update context with upgrade mode
    let mut update_context = UpdateContext::new()
        .with_upgrade(true) // Upgrade mode (ignore upper bounds)
        .with_event_sender(ctx.tx.clone());

    for package_name in package_names {
        update_context = update_context.add_package(package_name.clone());
    }

    // Execute upgrade
    let result = installer.update(update_context).await?;

    // Convert to report format
    let report = InstallReport {
        installed: result
            .installed_packages
            .iter()
            .map(|pkg| crate::types::PackageChange {
                name: pkg.name.clone(),
                from_version: None,
                to_version: Some(pkg.version.clone()),
                size: None,
            })
            .collect(),
        updated: result
            .updated_packages
            .iter()
            .map(|pkg| {
                crate::types::PackageChange {
                    name: pkg.name.clone(),
                    from_version: None, // TODO: Get previous version
                    to_version: Some(pkg.version.clone()),
                    size: None,
                }
            })
            .collect(),
        removed: result
            .removed_packages
            .iter()
            .map(|pkg| crate::types::PackageChange {
                name: pkg.name.clone(),
                from_version: Some(pkg.version.clone()),
                to_version: None,
                size: None,
            })
            .collect(),
        state_id: result.state_id,
        duration_ms: start.elapsed().as_millis() as u64,
    };

    ctx.tx.send(Event::UpgradeCompleted {
        packages: result
            .updated_packages
            .iter()
            .map(|pkg| pkg.name.clone())
            .collect(),
        state_id: result.state_id,
    });

    Ok(report)
}

/// Uninstall packages (delegates to install crate)
pub async fn uninstall(ctx: &OpsCtx, package_names: &[String]) -> Result<InstallReport, Error> {
    let start = Instant::now();

    if package_names.is_empty() {
        return Err(OpsError::NoPackagesSpecified.into());
    }

    ctx.tx.send(Event::UninstallStarting {
        packages: package_names.to_vec(),
    });

    // Create installer
    let config = InstallConfig::default();
    let mut installer = Installer::new(
        config,
        ctx.resolver.clone(),
        ctx.state.clone(),
        ctx.store.clone(),
    );

    // Build uninstall context
    let mut uninstall_context = UninstallContext::new().with_event_sender(ctx.tx.clone());

    for package_name in package_names {
        uninstall_context = uninstall_context.add_package(package_name.clone());
    }

    // Execute uninstallation
    let result = installer.uninstall(uninstall_context).await?;

    // Convert to report format
    let report = InstallReport {
        installed: Vec::new(), // No packages installed during uninstall
        updated: Vec::new(),   // No packages updated during uninstall
        removed: result
            .removed_packages
            .iter()
            .map(|pkg| crate::types::PackageChange {
                name: pkg.name.clone(),
                from_version: Some(pkg.version.clone()),
                to_version: None,
                size: None,
            })
            .collect(),
        state_id: result.state_id,
        duration_ms: start.elapsed().as_millis() as u64,
    };

    ctx.tx.send(Event::UninstallCompleted {
        packages: result
            .removed_packages
            .iter()
            .map(|pkg| pkg.name.clone())
            .collect(),
        state_id: result.state_id,
    });

    Ok(report)
}

/// Build package from recipe (delegates to builder crate)
pub async fn build(
    ctx: &OpsCtx,
    recipe_path: &Path,
    output_dir: Option<&Path>,
) -> Result<BuildReport, Error> {
    let start = Instant::now();

    if !recipe_path.exists() {
        return Err(OpsError::RecipeNotFound {
            path: recipe_path.display().to_string(),
        }
        .into());
    }

    if !recipe_path.extension().map_or(false, |ext| ext == "star") {
        return Err(OpsError::InvalidRecipe {
            path: recipe_path.display().to_string(),
            reason: "recipe must have .star extension".to_string(),
        }
        .into());
    }

    ctx.tx.send(Event::BuildStarting {
        package: "unknown".to_string(), // Will be determined from recipe
        version: Version::parse("0.0.0").unwrap_or_else(|_| Version::new(0, 0, 0)),
    });

    // Parse recipe to get package name and version
    let (package_name, package_version) = parse_recipe_metadata(recipe_path)?;

    // Create build context
    let output_directory = output_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let build_context = BuildContext::new(
        package_name.clone(),
        package_version.clone(),
        recipe_path.to_path_buf(),
        output_directory,
    )
    .with_event_sender(ctx.tx.clone());

    // Create builder with resolver and store
    let config = BuildConfig::default();
    let builder = spsv2_builder::Builder::with_config(config)
        .with_resolver(ctx.resolver.clone())
        .with_store(ctx.store.clone());

    // Execute build
    let result = builder.build(build_context).await?;

    let report = BuildReport {
        package: package_name,
        version: package_version,
        output_path: result.package_path,
        duration_ms: start.elapsed().as_millis() as u64,
        sbom_generated: !result.sbom_files.is_empty(),
    };

    ctx.tx.send(Event::BuildCompleted {
        package: report.package.clone(),
        version: report.version.clone(),
        path: report.output_path.clone(),
    });

    Ok(report)
}

/// Parse install requests from string specifications
fn parse_install_requests(specs: &[String]) -> Result<Vec<InstallRequest>, Error> {
    let mut requests = Vec::new();

    for spec in specs {
        if spec.ends_with(".sp") && Path::new(spec).exists() {
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

/// Parse recipe metadata to get package name and version
fn parse_recipe_metadata(recipe_path: &Path) -> Result<(String, Version), Error> {
    // This is a simplified implementation
    // In practice, this would parse the Starlark recipe to extract metadata

    let recipe_name = recipe_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    // For now, just use the filename as package name and default version
    Ok((
        recipe_name.to_string(),
        Version::parse("1.0.0").unwrap_or_else(|_| Version::new(1, 0, 0)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_install_requests() {
        let temp = tempdir().unwrap();

        // Create a test .sp file
        let sp_file = temp.path().join("test-1.0.0-1.arm64.sp");
        std::fs::write(&sp_file, b"test package").unwrap();

        let specs = vec![
            "curl>=8.0.0".to_string(),
            sp_file.display().to_string(),
            "jq==1.7.0".to_string(),
        ];

        let requests = parse_install_requests(&specs).unwrap();
        assert_eq!(requests.len(), 3);

        match &requests[0] {
            InstallRequest::Remote(spec) => assert_eq!(spec.name, "curl"),
            _ => panic!("Expected remote request"),
        }

        match &requests[1] {
            InstallRequest::LocalFile(path) => assert!(path.to_string().ends_with(".sp")),
            _ => panic!("Expected local file request"),
        }

        match &requests[2] {
            InstallRequest::Remote(spec) => assert_eq!(spec.name, "jq"),
            _ => panic!("Expected remote request"),
        }
    }

    #[test]
    fn test_parse_recipe_metadata() {
        let recipe_path = Path::new("/path/to/jq.rhai");
        let (name, version) = parse_recipe_metadata(recipe_path).unwrap();

        assert_eq!(name, "jq");
        assert_eq!(version, Version::parse("1.0.0").unwrap());
    }

    #[test]
    fn test_empty_install_specs() {
        let result = parse_install_requests(&[]);
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_invalid_package_spec() {
        let specs = vec!["invalid spec with spaces".to_string()];
        let result = parse_install_requests(&specs);
        assert!(result.is_err());
    }
}
