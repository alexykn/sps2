//! Build dependency installation

use super::core::BuildEnvironment;
use sps2_errors::{BuildError, Error};
use sps2_events::{AppEvent, FailureContext, GeneralEvent, LifecycleEvent};
use sps2_resolver::{InstalledPackage, ResolutionContext};
use sps2_state::StateManager;
use sps2_types::package::PackageSpec;
use sps2_types::Version;
use std::convert::TryFrom;
use std::time::Instant;

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

        let build_target_count = build_deps.len();

        // Get installed packages to check before resolving from repository
        let installed_packages = Self::get_installed_packages().await.unwrap_or_default();

        // Resolve build dependencies
        let mut resolution_context = ResolutionContext::new();
        for dep in build_deps {
            resolution_context = resolution_context.add_build_dep(dep);
        }

        let local_target_count = installed_packages.len();

        // Include installed packages to check before repository resolution
        resolution_context = resolution_context.with_installed_packages(installed_packages.clone());

        let resolve_start = Instant::now();
        self.send_event(AppEvent::Lifecycle(LifecycleEvent::resolver_started(
            0,
            build_target_count,
            local_target_count,
        )));

        let resolution = match resolver.resolve_with_sat(resolution_context).await {
            Ok(resolution) => resolution,
            Err(error) => {
                self.send_event(AppEvent::Lifecycle(LifecycleEvent::resolver_failed(
                    FailureContext::from_error(&error),
                    Vec::new(),
                )));
                return Err(error);
            }
        };

        let duration_ms = resolve_start.elapsed().as_millis();
        let duration_ms = u64::try_from(duration_ms).unwrap_or(u64::MAX);
        let mut downloaded_packages = 0usize;
        let mut reused_packages = 0usize;
        for node in resolution.nodes.values() {
            match node.action {
                sps2_resolver::NodeAction::Download => downloaded_packages += 1,
                sps2_resolver::NodeAction::Local => reused_packages += 1,
            }
        }
        self.send_event(AppEvent::Lifecycle(LifecycleEvent::resolver_completed(
            resolution.nodes.len(),
            downloaded_packages,
            reused_packages,
            duration_ms,
        )));

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
                self.send_event(AppEvent::General(GeneralEvent::debug(format!(
                    "{} {} is already installed in {}",
                    node.name,
                    node.version,
                    sps2_config::fixed_paths::LIVE_DIR
                ))));

                // Verify the package is installed
                self.verify_installed_package(&node.name, &node.version)
                    .await?;

                // Package verification completed - metrics removed as per architectural decision

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

        self.send_event(AppEvent::Lifecycle(LifecycleEvent::install_started(
            node.name.clone(),
            node.version.clone(),
        )));

        // Install the build dependency to the isolated deps prefix
        // This extracts the package contents to the build environment
        match &node.action {
            sps2_resolver::NodeAction::Download => {
                if let Some(url) = &node.url {
                    self.send_event(AppEvent::Lifecycle(LifecycleEvent::download_started(
                        url.clone(),
                        Some(format!("{}:{}", node.name, node.version)),
                        None,
                    )));

                    // Download the .sp file to a temporary location
                    let temp_dir = std::env::temp_dir();
                    let sp_filename = format!("{}-{}.sp", node.name, node.version);
                    let temp_sp_path = temp_dir.join(&sp_filename);

                    // Use NetClient to download the file with consistent retry logic
                    let default_tx = {
                        let (tx, _) = sps2_events::channel();
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
        let base_path = std::path::Path::new(sps2_config::fixed_paths::PREFIX);
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
        let base_path = std::path::Path::new(sps2_config::fixed_paths::PREFIX);
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
