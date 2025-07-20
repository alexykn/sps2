//! Package Information and Search Operations

use crate::{OpsCtx, PackageInfo, PackageStatus, SearchResult};
use sps2_errors::{Error, OpsError};
use sps2_events::{Event, EventEmitter};

/// List installed packages
///
/// # Errors
///
/// Returns an error if package listing fails.
pub async fn list_packages(ctx: &OpsCtx) -> Result<Vec<PackageInfo>, Error> {
    ctx.emit_event(Event::ListStarting);

    // Get installed packages from state
    let installed_packages = ctx.state.get_installed_packages().await?;

    let mut package_infos = Vec::new();

    for package in installed_packages {
        // Get package details from index
        let package_version = package.version();
        let index_entry = ctx
            .index
            .get_version(&package.name, &package_version.to_string());

        let (description, homepage, license, dependencies) = if let Some(entry) = index_entry {
            (
                entry.description.clone(),
                entry.homepage.clone(),
                entry.license.clone(),
                entry.dependencies.runtime.clone(),
            )
        } else {
            (None, None, None, Vec::new())
        };

        // Check if there's an available update
        let available_version =
            ctx.index
                .get_package_versions(&package.name)
                .and_then(|versions| {
                    versions
                        .first()
                        .and_then(|entry| sps2_types::Version::parse(&entry.version()).ok())
                });

        let status = match &available_version {
            Some(avail) if avail > &package_version => PackageStatus::Outdated,
            _ => PackageStatus::Installed,
        };

        // Get package size from state database
        let size = Some(u64::try_from(package.size).unwrap_or(0));

        let package_info = PackageInfo {
            name: package.name.clone(),
            version: Some(package_version),
            available_version,
            description,
            homepage,
            license,
            status,
            dependencies,
            size,
            arch: None, // TODO: Get actual architecture
            installed: true,
        };

        package_infos.push(package_info);
    }

    // Sort by name
    package_infos.sort_by(|a, b| a.name.cmp(&b.name));

    ctx.emit_event(Event::ListCompleted {
        count: package_infos.len(),
    });

    Ok(package_infos)
}

/// Get information about a specific package
///
/// # Errors
///
/// Returns an error if package information retrieval fails.
pub async fn package_info(ctx: &OpsCtx, package_name: &str) -> Result<PackageInfo, Error> {
    // Check if package is installed
    let installed_packages = ctx.state.get_installed_packages().await?;
    let installed_version = installed_packages
        .iter()
        .find(|pkg| pkg.name == package_name)
        .map(sps2_state::Package::version);

    // Get available versions from index
    let versions = ctx
        .index
        .get_package_versions(package_name)
        .ok_or_else(|| OpsError::PackageNotFound {
            package: package_name.to_string(),
        })?;

    let latest_entry = versions.first().ok_or_else(|| OpsError::PackageNotFound {
        package: package_name.to_string(),
    })?;

    let available_version = sps2_types::Version::parse(&latest_entry.version())?;

    let status = match &installed_version {
        Some(installed) => {
            match installed.cmp(&available_version) {
                std::cmp::Ordering::Equal => PackageStatus::Installed,
                std::cmp::Ordering::Less => PackageStatus::Outdated,
                std::cmp::Ordering::Greater => PackageStatus::Local, // Newer than available
            }
        }
        None => PackageStatus::Available,
    };

    // Get package size if installed
    let size = if installed_version.is_some() {
        // Find the installed package to get its size from state database
        installed_packages
            .iter()
            .find(|pkg| pkg.name == package_name)
            .map(|pkg| u64::try_from(pkg.size).unwrap_or(0))
    } else {
        None
    };

    let package_info = PackageInfo {
        name: package_name.to_string(),
        version: installed_version.clone(),
        available_version: Some(available_version),
        description: latest_entry.description.clone(),
        homepage: latest_entry.homepage.clone(),
        license: latest_entry.license.clone(),
        status,
        dependencies: latest_entry.dependencies.runtime.clone(),
        size,
        arch: None, // TODO: Get actual architecture
        installed: installed_version.is_some(),
    };

    Ok(package_info)
}

/// Search for packages
///
/// # Errors
///
/// Returns an error if package search fails.
pub async fn search_packages(ctx: &OpsCtx, query: &str) -> Result<Vec<SearchResult>, Error> {
    ctx.emit_event(Event::SearchStarting {
        query: query.to_string(),
    });

    // Search package names in index
    let package_names = ctx.index.search(query);

    let mut results = Vec::new();
    let installed_packages = ctx.state.get_installed_packages().await?;

    for package_name in package_names {
        if let Some(versions) = ctx.index.get_package_versions(package_name) {
            if let Some(latest) = versions.first() {
                if let Ok(version) = sps2_types::Version::parse(&latest.version()) {
                    let installed = installed_packages
                        .iter()
                        .any(|pkg| pkg.name == package_name);

                    results.push(SearchResult {
                        name: package_name.to_string(),
                        version,
                        description: latest.description.clone(),
                        homepage: latest.homepage.clone(),
                        installed,
                    });
                }
            }
        }
    }

    ctx.emit_event(Event::SearchCompleted {
        query: query.to_string(),
        count: results.len(),
    });

    Ok(results)
}
