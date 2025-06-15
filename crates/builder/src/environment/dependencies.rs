//! Build dependency installation

use super::core::BuildEnvironment;
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use sps2_resolver::{InstalledPackage, ResolutionContext};
use sps2_state::StateManager;
use sps2_store::StoredPackage;
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
            let is_empty_or_none = match &node.path {
                None => true,
                Some(path) => path.as_os_str().is_empty(),
            };

            if is_empty_or_none {
                // Already installed - link from store to deps directory
                self.send_event(Event::DebugLog {
                    message: format!(
                        "{} {} is already installed, linking from store",
                        node.name, node.version
                    ),
                    context: std::collections::HashMap::new(),
                });

                // Link the already-installed package from store to deps prefix
                self.link_installed_package_to_deps(&node.name, &node.version)
                    .await?;

                self.send_event(Event::PackageInstalled {
                    name: node.name.clone(),
                    version: node.version.clone(),
                    path: self.deps_prefix.display().to_string(),
                });

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

    /// Link an already-installed package from the store to the deps directory
    ///
    /// # Errors
    ///
    /// Returns an error if the package cannot be found in the store or linking fails.
    async fn link_installed_package_to_deps(
        &self,
        name: &str,
        version: &Version,
    ) -> Result<(), Error> {
        let Some(store) = &self.store else {
            return Err(BuildError::MissingBuildDep {
                name: "package store not configured".to_string(),
            }
            .into());
        };

        // Get package hash from state
        let base_path = std::path::Path::new("/opt/pm");
        let state = StateManager::new(base_path).await?;
        let hash_str = state
            .get_package_hash(name, &version.to_string())
            .await?
            .ok_or_else(|| BuildError::MissingBuildDep {
                name: format!("{} {}", name, version),
            })?;

        // Parse hash string
        let hash =
            sps2_hash::Hash::from_hex(&hash_str).map_err(|_| BuildError::MissingBuildDep {
                name: format!("invalid hash for {} {}", name, version),
            })?;

        // Get stored package
        let stored_package = StoredPackage::load(&store.package_path(&hash)).await?;

        // Create deps directory if it doesn't exist
        fs::create_dir_all(&self.deps_prefix).await?;

        // Copy package contents to deps directory with path replacement
        // We need to copy (not link) and replace /opt/pm/live paths with deps paths
        self.copy_package_with_path_replacement(&stored_package, &self.deps_prefix)
            .await?;

        Ok(())
    }

    /// Copy a package from store to deps with path replacement
    ///
    /// # Errors
    ///
    /// Returns an error if copying or path replacement fails.
    async fn copy_package_with_path_replacement(
        &self,
        stored_package: &StoredPackage,
        dest_dir: &Path,
    ) -> Result<(), Error> {
        let files_path = stored_package.files_path();
        let old_prefix = "/opt/pm/live";
        let new_prefix = self.deps_prefix.display().to_string();

        // Walk through all files in the package
        self.copy_directory_with_replacement(&files_path, dest_dir, old_prefix, &new_prefix)
            .await?;

        Ok(())
    }

    /// Recursively copy directory with path replacement in text files
    async fn copy_directory_with_replacement(
        &self,
        src_dir: &Path,
        dest_dir: &Path,
        old_prefix: &str,
        new_prefix: &str,
    ) -> Result<(), Error> {
        // Create destination directory
        fs::create_dir_all(dest_dir).await?;

        let mut entries = fs::read_dir(src_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            let src_path = entry.path();
            let file_name = entry.file_name();
            let dest_path = dest_dir.join(&file_name);

            if file_type.is_dir() {
                // Recursively copy subdirectory
                Box::pin(self.copy_directory_with_replacement(
                    &src_path, &dest_path, old_prefix, new_prefix,
                ))
                .await?;
            } else if file_type.is_symlink() {
                // Copy symlink as-is
                let target = fs::read_link(&src_path).await?;
                if let Err(e) = fs::symlink(&target, &dest_path).await {
                    // If symlink fails, try to remove and recreate
                    if dest_path.exists() {
                        fs::remove_file(&dest_path).await?;
                        fs::symlink(&target, &dest_path).await?;
                    } else {
                        return Err(e.into());
                    }
                }
            } else {
                // Regular file - first copy it
                fs::copy(&src_path, &dest_path).await?;

                // Copy file permissions
                let metadata = fs::metadata(&src_path).await?;
                let perms = metadata.permissions();
                fs::set_permissions(&dest_path, perms).await?;

                // Check if it's a dylib that needs path updates
                if let Some(ext) = dest_path.extension() {
                    if ext == "dylib" || dest_path.to_string_lossy().contains(".dylib.") {
                        self.update_dylib_paths(&dest_path, old_prefix, new_prefix)
                            .await?;
                    }
                }

                // Check if it's a text file that needs path replacement
                if self.is_text_file(&src_path).await? {
                    // Try to do in-place text replacement
                    match fs::read_to_string(&dest_path).await {
                        Ok(content) => {
                            if content.contains(old_prefix) {
                                let new_content = content.replace(old_prefix, new_prefix);
                                fs::write(&dest_path, new_content).await?;
                            }
                        }
                        Err(_) => {
                            // Not a valid UTF-8 file, skip text replacement
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Check if a file is likely a text file that needs path replacement
    async fn is_text_file(&self, path: &Path) -> Result<bool, Error> {
        // Check by extension first
        if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            let text_extensions = [
                // Headers
                "h", "hpp", "hxx", "hh", "h++", // Source files
                "c", "cpp", "cc", "cxx", "c++", "m", "mm", // Build files
                "pc", "cmake", "am", "in", "ac", "mk", "make", // Scripts
                "py", "pl", "rb", "sh", "bash", "zsh", "fish", // Config files
                "conf", "cfg", "ini", "toml", "yaml", "yml", "json", // Other text files
                "txt", "md", "rst", "xml", "html", "css", "js",
                // Library files that might have paths
                "la", "prl",
            ];

            if text_extensions.contains(&ext_str.as_str()) {
                return Ok(true);
            }
        }

        // Check for files without extension that might be scripts
        if let Some(file_name) = path.file_name() {
            let name_str = file_name.to_string_lossy();
            // Common script names without extension
            if name_str == "configure" || name_str.starts_with("config.") {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Update paths in a dylib file using install_name_tool
    async fn update_dylib_paths(
        &self,
        dylib_path: &Path,
        old_prefix: &str,
        new_prefix: &str,
    ) -> Result<(), Error> {
        // First, get the current install name and dependencies
        let output = Command::new("otool")
            .args(["-L", &dylib_path.to_string_lossy()])
            .output()
            .await?;

        if !output.status.success() {
            // If otool fails, skip this file
            return Ok(());
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();

        if lines.is_empty() {
            return Ok(());
        }

        // Update install name
        self.update_dylib_install_name(&lines, dylib_path, old_prefix, new_prefix)
            .await?;

        // Update dependency paths
        self.update_dylib_dependencies(&lines, dylib_path, old_prefix, new_prefix)
            .await?;

        // Update RPATHs
        self.update_dylib_rpaths(dylib_path, old_prefix, new_prefix)
            .await?;

        Ok(())
    }

    /// Update the install name of a dylib
    async fn update_dylib_install_name(
        &self,
        lines: &[&str],
        dylib_path: &Path,
        old_prefix: &str,
        new_prefix: &str,
    ) -> Result<(), Error> {
        // First line after the header is the install name (for dylibs)
        // Format: "    /opt/pm/live/lib/libgmp.10.dylib (compatibility version ...)"
        if lines.len() > 1 {
            let first_dep = lines[1].trim();
            if let Some(space_pos) = first_dep.find(" (") {
                let install_name = &first_dep[..space_pos];
                if install_name.contains(old_prefix) {
                    let new_install_name = install_name.replace(old_prefix, new_prefix);

                    // Update the install name
                    let result = Command::new("install_name_tool")
                        .args(["-id", &new_install_name, &dylib_path.to_string_lossy()])
                        .output()
                        .await?;

                    if !result.status.success() {
                        self.send_event(Event::Warning {
                            message: format!(
                                "Failed to update install name for {}: {}",
                                dylib_path.display(),
                                String::from_utf8_lossy(&result.stderr)
                            ),
                            context: None,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Update dependency paths in a dylib
    async fn update_dylib_dependencies(
        &self,
        lines: &[&str],
        dylib_path: &Path,
        old_prefix: &str,
        new_prefix: &str,
    ) -> Result<(), Error> {
        for line in lines.iter().skip(1) {
            let trimmed = line.trim();
            if let Some(space_pos) = trimmed.find(" (") {
                let dep_path = &trimmed[..space_pos];
                if dep_path.contains(old_prefix) {
                    let new_dep_path = dep_path.replace(old_prefix, new_prefix);

                    // Update the dependency path
                    let result = Command::new("install_name_tool")
                        .args([
                            "-change",
                            dep_path,
                            &new_dep_path,
                            &dylib_path.to_string_lossy(),
                        ])
                        .output()
                        .await?;

                    if !result.status.success() {
                        self.send_event(Event::Warning {
                            message: format!(
                                "Failed to update dependency {} in {}: {}",
                                dep_path,
                                dylib_path.display(),
                                String::from_utf8_lossy(&result.stderr)
                            ),
                            context: None,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Update RPATHs in a dylib
    async fn update_dylib_rpaths(
        &self,
        dylib_path: &Path,
        old_prefix: &str,
        new_prefix: &str,
    ) -> Result<(), Error> {
        let rpath_output = Command::new("otool")
            .args(["-l", &dylib_path.to_string_lossy()])
            .output()
            .await?;

        if rpath_output.status.success() {
            let rpath_str = String::from_utf8_lossy(&rpath_output.stdout);
            let mut lines = rpath_str.lines();

            while let Some(line) = lines.next() {
                if line.contains("LC_RPATH") {
                    // Skip the cmdsize line
                    let _ = lines.next();
                    // Get the path line
                    if let Some(path_line) = lines.next() {
                        if path_line.contains("path ") {
                            if let Some(path_start) = path_line.find("path ") {
                                let path_part = &path_line[path_start + 5..];
                                if let Some(space_pos) = path_part.find(" (") {
                                    let rpath = &path_part[..space_pos];
                                    if rpath.contains(old_prefix) {
                                        let new_rpath = rpath.replace(old_prefix, new_prefix);

                                        // First delete the old rpath
                                        let _ = Command::new("install_name_tool")
                                            .args([
                                                "-delete_rpath",
                                                rpath,
                                                &dylib_path.to_string_lossy(),
                                            ])
                                            .output()
                                            .await;

                                        // Then add the new one
                                        let _ = Command::new("install_name_tool")
                                            .args([
                                                "-add_rpath",
                                                &new_rpath,
                                                &dylib_path.to_string_lossy(),
                                            ])
                                            .output()
                                            .await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
