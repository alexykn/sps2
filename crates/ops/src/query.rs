//! Package Information and Search Operations

use crate::{OpsCtx, PackageInfo, PackageStatus, SearchResult};
use sps2_errors::{Error, OpsError};
use sps2_events::{
    events::{PackageOperation, PackageOutcome},
    AppEvent, EventEmitter, PackageEvent,
};
use sps2_hash::Hash;
use sps2_store::StoredPackage;

/// List installed packages
///
/// # Errors
///
/// Returns an error if package listing fails.
pub async fn list_packages(ctx: &OpsCtx) -> Result<Vec<PackageInfo>, Error> {
    let _correlation = ctx.push_correlation("query:list");

    ctx.emit(AppEvent::Package(PackageEvent::OperationStarted {
        operation: PackageOperation::List,
    }));

    // Get installed packages from state
    let installed_packages = ctx.state.get_installed_packages().await?;

    let mut package_infos = Vec::new();

    for package in installed_packages {
        // Get package details from index
        let package_version = package.version();
        let index_entry = ctx
            .index
            .get_version(&package.name, &package_version.to_string());

        let (mut description, mut homepage, mut license, mut dependencies) =
            if let Some(entry) = index_entry {
                (
                    entry.description.clone(),
                    entry.homepage.clone(),
                    entry.license.clone(),
                    entry.dependencies.runtime.clone(),
                )
            } else {
                (None, None, None, Vec::new())
            };

        if description.is_none()
            || homepage.is_none()
            || license.is_none()
            || dependencies.is_empty()
        {
            if let Ok(hash) = Hash::from_hex(&package.hash) {
                let package_path = ctx.store.package_path(&hash);
                if let Ok(stored) = StoredPackage::load(&package_path).await {
                    let manifest = stored.manifest();
                    if description.is_none() {
                        description.clone_from(&manifest.package.description);
                    }
                    if homepage.is_none() {
                        homepage.clone_from(&manifest.package.homepage);
                    }
                    if license.is_none() {
                        license.clone_from(&manifest.package.license);
                    }
                    if dependencies.is_empty() {
                        dependencies.clone_from(&manifest.dependencies.runtime);
                    }
                }
            }
        }

        // Check if there's an available update
        let available_version = ctx
            .index
            .get_package_versions_with_strings(&package.name)
            .and_then(|versions| {
                versions
                    .first()
                    .and_then(|(ver_str, _)| sps2_types::Version::parse(ver_str).ok())
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

    ctx.emit(AppEvent::Package(PackageEvent::OperationCompleted {
        operation: PackageOperation::List,
        outcome: PackageOutcome::List {
            total: package_infos.len(),
        },
    }));

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

    // Get available versions from index (with version strings)
    let versions = ctx
        .index
        .get_package_versions_with_strings(package_name)
        .ok_or_else(|| OpsError::PackageNotFound {
            package: package_name.to_string(),
        })?;

    let (latest_version_str, latest_entry) =
        versions.first().ok_or_else(|| OpsError::PackageNotFound {
            package: package_name.to_string(),
        })?;

    let available_version = sps2_types::Version::parse(latest_version_str)?;

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
    let _correlation = ctx.push_correlation(format!("query:search:{query}"));

    ctx.emit(AppEvent::Package(PackageEvent::OperationStarted {
        operation: PackageOperation::Search,
    }));

    // Search package names in index
    let package_names = ctx.index.search(query);

    let mut results = Vec::new();
    let installed_packages = ctx.state.get_installed_packages().await?;

    for package_name in package_names {
        if let Some(versions) = ctx.index.get_package_versions_with_strings(package_name) {
            if let Some((version_str, latest)) = versions.first() {
                if let Ok(version) = sps2_types::Version::parse(version_str) {
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

    ctx.emit(AppEvent::Package(PackageEvent::OperationCompleted {
        operation: PackageOperation::Search,
        outcome: PackageOutcome::Search {
            query: query.to_string(),
            total: results.len(),
        },
    }));

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpsContextBuilder;
    use sps2_builder::Builder;
    use sps2_config::Config;
    use sps2_install::{AtomicInstaller, InstallContext, PreparedPackage};
    use sps2_net::{NetClient, NetConfig};
    use sps2_resolver::{PackageId, ResolvedNode, Resolver};
    use sps2_state::StateManager;
    use sps2_store::{create_package, PackageStore};
    use sps2_types::{Arch, Manifest, Version};
    use std::collections::HashMap;
    use tempfile::TempDir;
    use tokio::fs as afs;

    async fn make_package(
        store: &PackageStore,
        name: &str,
        version: &str,
        description: &str,
    ) -> (sps2_hash::Hash, std::path::PathBuf, u64) {
        let td = TempDir::new().expect("package tempdir");
        let src = td.path().join("src");
        afs::create_dir_all(&src).await.expect("src dir");

        let version_parsed = Version::parse(version).expect("version");
        let mut manifest = Manifest::new(name.to_string(), &version_parsed, 1, &Arch::Arm64);
        manifest.package.description = Some(description.to_string());
        sps2_store::manifest_io::write_manifest(&src.join("manifest.toml"), &manifest)
            .await
            .expect("write manifest");

        let content_dir = src.join("opt/pm/live/share");
        afs::create_dir_all(&content_dir)
            .await
            .expect("content dir");
        afs::write(content_dir.join("file.txt"), description.as_bytes())
            .await
            .expect("write file");

        let sp_path = td.path().join("pkg.sp");
        create_package(&src, &sp_path)
            .await
            .expect("create package");

        let stored = store.add_package(&sp_path).await.expect("add package");
        let hash = stored.hash().expect("hash");
        let path = store.package_path(&hash);
        let size = afs::metadata(&sp_path).await.expect("metadata").len();
        (hash, path, size)
    }

    #[tokio::test]
    async fn list_packages_uses_manifest_description_when_index_missing() {
        let temp_dir = TempDir::new().expect("ops tempdir");
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");
        afs::create_dir_all(&state_dir).await.expect("state dir");
        afs::create_dir_all(&store_dir).await.expect("store dir");

        let state = StateManager::new(&state_dir).await.expect("state manager");
        let store = PackageStore::new(store_dir.clone());

        let description = "Demo package description";
        let (hash, store_path, size) = make_package(&store, "demo", "1.2.3", description).await;

        let mut atomic = AtomicInstaller::new(state.clone(), store.clone())
            .await
            .expect("atomic installer");
        let pkg_id = PackageId::new("demo".to_string(), Version::parse("1.2.3").unwrap());
        let mut resolved_nodes = HashMap::new();
        resolved_nodes.insert(
            pkg_id.clone(),
            ResolvedNode::local(
                "demo".to_string(),
                pkg_id.version.clone(),
                store_path.clone(),
                vec![],
            ),
        );
        let mut prepared = HashMap::new();
        prepared.insert(
            pkg_id.clone(),
            PreparedPackage {
                hash,
                size,
                store_path,
                is_local: true,
                package_hash: None,
            },
        );
        let install_ctx = InstallContext {
            packages: vec![],
            local_files: vec![],
            force: false,
            force_download: false,
            event_sender: None,
        };
        atomic
            .install(&install_ctx, &resolved_nodes, Some(&prepared))
            .await
            .expect("install package");

        let (tx, _rx) = sps2_events::channel();
        let config = Config::default();
        let index = sps2_index::IndexManager::new(temp_dir.path().join("index"));
        let net = NetClient::new_without_proxies(NetConfig::default()).expect("net client");
        let resolver_instance = Resolver::with_events(index.clone(), tx.clone());
        let builder = Builder::new();

        let ctx = OpsContextBuilder::new()
            .with_state(state)
            .with_store(store)
            .with_index(index)
            .with_net(net)
            .with_resolver(resolver_instance)
            .with_builder(builder)
            .with_event_sender(tx)
            .with_config(config)
            .build()
            .expect("ops ctx");

        let packages = list_packages(&ctx).await.expect("list packages");
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "demo");
        assert_eq!(packages[0].description.as_deref(), Some(description));
    }
}
