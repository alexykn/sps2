//! Build dependency installation

use super::core::BuildEnvironment;
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use sps2_resolver::{InstalledPackage, ResolutionContext};
use sps2_state::StateManager;
use sps2_types::package::PackageSpec;
use sps2_types::Version;
use std::path::Path;
use tokio::fs;
use tokio::process::Command;

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

        self.send_event(Event::DependencyResolved {
            package: self.context.name.clone(),
            version: self.context.version.clone(),
            count: 1, // Single package resolved
        });

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
            match &node.path {
                None => {
                    // No path means already installed
                    self.send_event(Event::DebugLog {
                        message: format!(
                            "{} {} is already installed, skipping",
                            node.name, node.version
                        ),
                        context: std::collections::HashMap::new(),
                    });
                    return Ok(());
                }
                Some(path) if path.as_os_str().is_empty() => {
                    // Empty path also means already installed
                    self.send_event(Event::DebugLog {
                        message: format!(
                            "{} {} is already installed, skipping",
                            node.name, node.version
                        ),
                        context: std::collections::HashMap::new(),
                    });
                    return Ok(());
                }
                _ => {
                    // Has a real path, so it's a local file to install
                }
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

        self.send_event(Event::PackageInstalling {
            name: node.name.clone(),
            version: node.version.clone(),
        });

        // Install the build dependency to the isolated deps prefix
        // This extracts the package contents to the build environment
        match &node.action {
            sps2_resolver::NodeAction::Download => {
                if let Some(url) = &node.url {
                    self.send_event(Event::DownloadStarted {
                        url: url.clone(),
                        size: None,
                    });

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

                    // Extract the .sp file to the deps prefix
                    self.extract_sp_package(&temp_sp_path, &self.deps_prefix)
                        .await?;

                    // Clean up temporary file
                    if temp_sp_path.exists() {
                        let _ = tokio::fs::remove_file(&temp_sp_path).await;
                    }

                    self.send_event(Event::PackageInstalled {
                        name: node.name.clone(),
                        version: node.version.clone(),
                        path: self.deps_prefix.display().to_string(),
                    });
                }
            }
            sps2_resolver::NodeAction::Local => {
                if let Some(path) = &node.path {
                    // Extract local .sp file to deps prefix
                    self.extract_sp_package(path, &self.deps_prefix).await?;

                    self.send_event(Event::PackageInstalled {
                        name: node.name.clone(),
                        version: node.version.clone(),
                        path: self.deps_prefix.display().to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Extract .sp package to destination directory
    ///
    /// # Errors
    ///
    /// Returns an error if package extraction fails.
    async fn extract_sp_package(&self, sp_path: &Path, dest_dir: &Path) -> Result<(), Error> {
        // Create destination directory
        fs::create_dir_all(dest_dir).await?;

        // Extract .sp package (compressed tar archive)
        // First decompress with zstd, then extract with tar
        let temp_dir = std::env::temp_dir();
        let tar_filename = format!(
            "{}.tar",
            sp_path.file_stem().unwrap_or_default().to_string_lossy()
        );
        let temp_tar_path = temp_dir.join(&tar_filename);

        // Decompress with zstd using async-compression crate
        {
            use async_compression::tokio::bufread::ZstdDecoder;
            use tokio::fs::File;
            use tokio::io::{AsyncWriteExt, BufReader, BufWriter};

            let input_file = File::open(sp_path).await?;
            let output_file = File::create(&temp_tar_path).await?;

            // Create zstd decoder
            let mut decoder = ZstdDecoder::new(BufReader::new(input_file));

            // Copy decompressed data to output file
            let mut writer = BufWriter::new(output_file);
            tokio::io::copy(&mut decoder, &mut writer)
                .await
                .map_err(|e| BuildError::ExtractionFailed {
                    message: format!("zstd decompression failed: {e}"),
                })?;

            // Ensure all data is written
            writer.flush().await?;
        }

        // Extract tar archive, only extracting the 'files/' directory to preserve package structure
        let tar_output = Command::new("tar")
            .args([
                "--extract",
                "--file",
                &temp_tar_path.display().to_string(),
                "--directory",
                &dest_dir.display().to_string(),
                "--strip-components=1", // Remove 'files/' prefix
                "files/",
            ])
            .output()
            .await?;

        if !tar_output.status.success() {
            return Err(BuildError::ExtractionFailed {
                message: format!(
                    "tar extraction failed: {}",
                    String::from_utf8_lossy(&tar_output.stderr)
                ),
            }
            .into());
        }

        // Clean up temporary tar file
        if temp_tar_path.exists() {
            let _ = fs::remove_file(&temp_tar_path).await;
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
}
