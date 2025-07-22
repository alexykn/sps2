//! Build dependency installation

use super::core::BuildEnvironment;
use sps2_errors::{BuildError, Error};
use sps2_events::{AppEvent, DownloadEvent, GeneralEvent, InstallEvent, ResolverEvent};
use sps2_resolver::{InstalledPackage, ResolutionContext};
use sps2_state::StateManager;
use sps2_types::package::PackageSpec;
use sps2_types::Version;

impl BuildEnvironment {
    /// Setup build dependencies
    ///
    /// # Errors
    ///
    /// Returns an error if dependency resolution fails or build dependencies cannot be installed.
    pub async fn setup_dependencies(&mut self, build_deps: Vec<PackageSpec>) -> Result<(), Error> {
        if build_deps.is_empty() {
            return Ok(());
        }

        let Some(resolver) = &self.resolver else {
            return Err(BuildError::MissingBuildDep {
                name: "resolver configuration".to_string(),
            }
            .into());
        };

        self.send_event(AppEvent::Resolver(ResolverEvent::ResolutionCompleted {
            total_packages: 1,
            execution_batches: 1,
            duration_ms: 0, // TODO: track actual duration
            packages_resolved: vec![format!("{}:{}", self.context.name, self.context.version)],
        }));

        // Get installed packages to check before resolving from repository
        let installed_packages = Self::get_installed_packages().await.unwrap_or_default();

        // Resolve build dependencies
        let mut resolution_context = ResolutionContext::new();
        for dep in build_deps {
            resolution_context = resolution_context.add_build_dep(dep);
        }

        // Include installed packages to check before repository resolution
        resolution_context = resolution_context.with_installed_packages(installed_packages);

        let resolution = resolver.resolve_with_sat(resolution_context).await?;

        // Install build dependencies to deps prefix
        for node in resolution.packages_in_order() {
            // Install all resolved build dependencies to the isolated deps prefix
            self.install_build_dependency(node).await?;
        }

        // Update environment for build deps
        self.setup_build_deps_environment();

        Ok(())
    }

    /// Install a build dependency to isolated prefix
    ///
    /// # Errors
    ///
    /// Returns an error if the installer or store is not configured, or if installation fails.
    async fn install_build_dependency(
        &self,
        node: &sps2_resolver::ResolvedNode,
    ) -> Result<(), Error> {
        // Check if this is an already-installed package (marked by resolver with Local action and empty path)
        if matches!(&node.action, sps2_resolver::NodeAction::Local) {
            let is_empty_or_none = match &node.path {
                None => true,
                Some(path) => path.as_os_str().is_empty(),
            };

            if is_empty_or_none {
                // Already installed - just verify it exists
                self.send_event(AppEvent::General(GeneralEvent::debug(
                    &format!(
                        "{} {} is already installed in /opt/pm/live",
                        node.name, node.version
                    )
                )));

                // Verify the package is installed
                self.verify_installed_package(&node.name, &node.version)
                    .await?;

                self.send_event(AppEvent::Install(InstallEvent::Completed {
                    package: node.name.clone(),
                    version: node.version.clone(),
                    installed_files: 0, // TODO: track actual file count
                    install_path: std::path::PathBuf::from("/opt/pm/live"),
                    duration: std::time::Duration::from_secs(0), // TODO: track actual duration
                    disk_usage: 0, // TODO: track actual disk usage
                }));

                return Ok(());
            }
        }

        let Some(_installer) = &self.installer else {
            return Err(BuildError::MissingBuildDep {
                name: "installer not configured".to_string(),
            }
            .into());
        };

        let Some(_store) = &self.store else {
            return Err(BuildError::MissingBuildDep {
                name: "package store not configured".to_string(),
            }
            .into());
        };

        let Some(net_client) = &self.net else {
            return Err(BuildError::MissingBuildDep {
                name: "network client not configured".to_string(),
            }
            .into());
        };

        self.send_event(AppEvent::Install(InstallEvent::Started {
            package: node.name.clone(),
            version: node.version.clone(),
            install_path: std::path::PathBuf::from("/opt/pm/live"),
            force_reinstall: false,
        }));

        // Install the build dependency to the isolated deps prefix
        // This extracts the package contents to the build environment
        match &node.action {
            sps2_resolver::NodeAction::Download => {
                if let Some(url) = &node.url {
                    self.send_event(AppEvent::Download(DownloadEvent::Started {
                        url: url.clone(),
                        package: Some(format!("{}:{}", node.name, node.version)),
                        total_size: None,
                        supports_resume: false,
                        connection_time: std::time::Duration::from_secs(0), // TODO: track actual connection time
                    }));

                    // Download the .sp file to a temporary location
                    let temp_dir = std::env::temp_dir();
                    let sp_filename = format!("{}-{}.sp", node.name, node.version);
                    let temp_sp_path = temp_dir.join(&sp_filename);

                    // Use NetClient to download the file with consistent retry logic
                    let default_tx = {
                        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
                        tx
                    };
                    let event_sender = self.context.event_sender.as_ref().unwrap_or(&default_tx);
                    let bytes = sps2_net::fetch_bytes(net_client, url, event_sender)
                        .await
                        .map_err(|_e| BuildError::FetchFailed { url: url.clone() })?;

                    tokio::fs::write(&temp_sp_path, &bytes).await?;

                    // Clean up temporary file
                    if temp_sp_path.exists() {
                        let _ = tokio::fs::remove_file(&temp_sp_path).await;
                    }

                    // We don't extract to deps anymore - package should be installed to /opt/pm/live
                    return Err(BuildError::MissingBuildDep {
                        name: format!(
                            "{} {} needs to be installed via 'sps2 install'",
                            node.name, node.version
                        ),
                    }
                    .into());
                }
            }
            sps2_resolver::NodeAction::Local => {
                if let Some(_path) = &node.path {
                    // We don't extract to deps anymore - package should be installed to /opt/pm/live
                    return Err(BuildError::MissingBuildDep {
                        name: format!(
                            "{} {} needs to be installed via 'sps2 install'",
                            node.name, node.version
                        ),
                    }
                    .into());
                }
            }
        }

        Ok(())
    }

    /// Get currently installed packages from system state
    async fn get_installed_packages() -> Result<Vec<InstalledPackage>, Error> {
        // Create a minimal state manager to check installed packages
        let base_path = std::path::Path::new("/opt/pm");
        let state = StateManager::new(base_path).await?;

        let packages = state.get_installed_packages().await?;

        let mut installed = Vec::new();
        for pkg in packages {
            let version = Version::parse(&pkg.version)?;
            installed.push(InstalledPackage::new(pkg.name, version));
        }

        Ok(installed)
    }

    /// Verify an already-installed package exists
    ///
    /// # Errors
    ///
    /// Returns an error if the package is not installed.
    async fn verify_installed_package(&self, name: &str, version: &Version) -> Result<(), Error> {
        // Check if package is installed using state manager
        let base_path = std::path::Path::new("/opt/pm");
        let state = StateManager::new(base_path).await?;

        // Get all installed packages
        let installed = state.get_installed_packages().await?;

        // Check if our package is in the list
        let is_installed = installed
            .iter()
            .any(|pkg| pkg.name == name && pkg.version == version.to_string());

        if !is_installed {
            return Err(BuildError::MissingBuildDep {
                name: format!("{name} {version} is not installed"),
            }
            .into());
        }

        Ok(())
    }
}
