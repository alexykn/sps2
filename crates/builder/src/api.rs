//! Builder API for Starlark recipes

use crate::{BuildCommandResult, BuildEnvironment};
use spsv2_errors::{BuildError, Error};
use spsv2_hash::Hash;
use spsv2_net::{NetClient, NetConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Builder API exposed to Starlark recipes
#[derive(Clone)]
pub struct BuilderApi {
    /// Working directory for source extraction
    working_dir: PathBuf,
    /// Downloaded files
    downloads: HashMap<String, PathBuf>,
    /// Network client for downloads
    net_client: NetClient,
    /// Whether network access is allowed
    allow_network: bool,
    /// SBOM generation enabled
    auto_sbom: bool,
    /// SBOM exclusion patterns
    sbom_excludes: Vec<String>,
}

impl BuilderApi {
    /// Create new builder API
    ///
    /// # Errors
    ///
    /// Returns an error if the network client cannot be created.
    pub fn new(working_dir: PathBuf) -> Result<Self, Error> {
        Ok(Self {
            working_dir,
            downloads: HashMap::new(),
            net_client: NetClient::new(NetConfig::default())?,
            allow_network: false,
            auto_sbom: true,
            sbom_excludes: vec![
                "*.dSYM".to_string(),
                "*.pdb".to_string(),
                "*.a".to_string(),
                "*.la".to_string(),
            ],
        })
    }

    /// Allow network access during build
    #[must_use]
    pub fn allow_network(&mut self, allow: bool) -> &mut Self {
        self.allow_network = allow;
        self
    }

    /// Download and verify a file
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Network access is disabled
    /// - The URL is invalid
    /// - The download fails
    /// - The file hash doesn't match the expected hash
    pub async fn fetch(&mut self, url: &str, expected_hash: &str) -> Result<PathBuf, Error> {
        if !self.allow_network {
            return Err(BuildError::NetworkDisabled {
                url: url.to_string(),
            }
            .into());
        }

        // Check if already downloaded
        if let Some(path) = self.downloads.get(url) {
            return Ok(path.clone());
        }

        // Extract filename from URL
        let filename = url
            .split('/')
            .next_back()
            .ok_or_else(|| BuildError::InvalidUrl {
                url: url.to_string(),
            })?;

        let download_path = self.working_dir.join(filename);

        // Download file using the download module
        // For builder, we don't have an event sender, so we'll use the client directly
        let response = self.net_client.get(url).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|_e| BuildError::FetchFailed {
                url: url.to_string(),
            })?;
        fs::write(&download_path, &bytes).await?;

        // Verify hash
        let actual_hash = Hash::hash_file(&download_path).await?;
        if actual_hash.to_hex() != expected_hash {
            fs::remove_file(&download_path).await?;
            return Err(BuildError::HashMismatch {
                file: filename.to_string(),
                expected: expected_hash.to_string(),
                actual: actual_hash.to_hex(),
            }
            .into());
        }

        self.downloads
            .insert(url.to_string(), download_path.clone());
        Ok(download_path)
    }

    /// Apply a patch file
    ///
    /// # Errors
    ///
    /// Returns an error if the patch command fails.
    pub async fn apply_patch(
        &self,
        patch_path: &Path,
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        env.execute_command(
            "patch",
            &["-p1", "-i", &patch_path.display().to_string()],
            Some(&self.working_dir),
        )
        .await
    }

    /// Configure with autotools
    ///
    /// # Errors
    ///
    /// Returns an error if the configure or make commands fail.
    pub async fn autotools(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Run ./configure with PREFIX automatically added
        let mut configure_args = args.to_vec();
        let staging_dir = env.staging_dir().display().to_string();
        if !configure_args
            .iter()
            .any(|arg| arg.starts_with("--prefix="))
        {
            configure_args.insert(0, format!("--prefix={staging_dir}"));
        }

        env.execute_command(
            "sh",
            &["-c", &format!("./configure {}", configure_args.join(" "))],
            Some(&self.working_dir),
        )
        .await?;

        // Run make
        let jobs = env
            .env_vars()
            .get("JOBS")
            .unwrap_or(&"1".to_string())
            .clone();
        env.execute_command("make", &["-j", &jobs], Some(&self.working_dir))
            .await?;

        // Run make install
        env.execute_command("make", &["install"], Some(&self.working_dir))
            .await
    }

    /// Configure with `CMake`
    ///
    /// # Errors
    ///
    /// Returns an error if the cmake or make commands fail.
    pub async fn cmake(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Create build directory
        let build_dir = self.working_dir.join("build");
        fs::create_dir_all(&build_dir).await?;

        // Run cmake with CMAKE_INSTALL_PREFIX
        let mut cmake_args = vec!["..".to_string()];
        let staging_dir = env.staging_dir().display().to_string();

        // Add CMAKE_INSTALL_PREFIX if not already specified
        if !args.iter().any(|arg| arg.contains("CMAKE_INSTALL_PREFIX")) {
            cmake_args.push(format!("-DCMAKE_INSTALL_PREFIX={staging_dir}"));
        }
        cmake_args.extend(args.iter().cloned());

        let arg_strs: Vec<&str> = cmake_args.iter().map(String::as_str).collect();
        env.execute_command("cmake", &arg_strs, Some(&build_dir))
            .await?;

        // Run make
        let jobs = env
            .env_vars()
            .get("JOBS")
            .unwrap_or(&"1".to_string())
            .clone();
        env.execute_command("make", &["-j", &jobs], Some(&build_dir))
            .await?;

        // Run make install
        env.execute_command("make", &["install"], Some(&build_dir))
            .await
    }

    /// Configure with Meson
    ///
    /// # Errors
    ///
    /// Returns an error if the meson commands fail.
    pub async fn meson(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Setup meson
        let build_dir = self.working_dir.join("build");
        let staging_dir = env.staging_dir().display().to_string();

        let mut setup_args = vec!["setup".to_string(), build_dir.display().to_string()];

        // Add prefix if not already specified
        if !args.iter().any(|arg| arg.contains("--prefix")) {
            setup_args.push(format!("--prefix={staging_dir}"));
        }
        setup_args.extend(args.iter().cloned());

        let arg_strs: Vec<&str> = setup_args.iter().map(String::as_str).collect();
        env.execute_command("meson", &arg_strs, Some(&self.working_dir))
            .await?;

        // Compile
        env.execute_command("meson", &["compile"], Some(&build_dir))
            .await?;

        // Install
        env.execute_command("meson", &["install"], Some(&build_dir))
            .await
    }

    /// Build with Cargo
    ///
    /// # Errors
    ///
    /// Returns an error if the cargo command fails.
    pub async fn cargo(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Extract source archive first if needed
        self.extract_downloads().await?;

        let mut cargo_args = vec!["build", "--release"];
        let arg_strs: Vec<&str> = args.iter().map(String::as_str).collect();
        cargo_args.extend(arg_strs);

        env.execute_command("cargo", &cargo_args, Some(&self.working_dir))
            .await?;

        // Install binaries to staging directory
        let staging_bin = env.staging_dir().join("bin");
        fs::create_dir_all(&staging_bin).await?;

        // Copy target/release/* to staging/bin (simplified for now)
        let target_dir = self.working_dir.join("target/release");
        if target_dir.exists() {
            let mut entries = fs::read_dir(&target_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_file() && !path.extension().map_or(false, |ext| ext == "d") {
                    let filename = path.file_name().unwrap();
                    fs::copy(&path, staging_bin.join(filename)).await?;
                }
            }
        }

        Ok(BuildCommandResult {
            success: true,
            exit_code: Some(0),
            stdout: "Cargo build completed".to_string(),
            stderr: String::new(),
        })
    }

    /// Run configure step only
    ///
    /// # Errors
    ///
    /// Returns an error if the configure command fails.
    pub async fn configure(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        // Extract source archive first if needed
        self.extract_downloads().await?;

        // Add prefix if not already specified
        let mut configure_args = args.to_vec();
        let staging_dir = env.staging_dir().display().to_string();
        if !configure_args
            .iter()
            .any(|arg| arg.starts_with("--prefix="))
        {
            configure_args.insert(0, format!("--prefix={staging_dir}"));
        }

        env.execute_command(
            "sh",
            &["-c", &format!("./configure {}", configure_args.join(" "))],
            Some(&self.working_dir),
        )
        .await
    }

    /// Run make step only
    ///
    /// # Errors
    ///
    /// Returns an error if the make command fails.
    pub async fn make(
        &self,
        args: &[String],
        env: &BuildEnvironment,
    ) -> Result<BuildCommandResult, Error> {
        let mut make_args = vec!["make"];
        let arg_strs: Vec<&str> = args.iter().map(String::as_str).collect();
        make_args.extend(arg_strs);

        env.execute_command("make", &make_args[1..], Some(&self.working_dir))
            .await
    }

    /// Install to staging directory
    ///
    /// # Errors
    ///
    /// Returns an error if the install command fails.
    pub async fn install(&self, env: &BuildEnvironment) -> Result<BuildCommandResult, Error> {
        // Check if we're in a build subdirectory (CMake/Meson)
        let build_dir = self.working_dir.join("build");
        let install_dir = if build_dir.exists() {
            &build_dir
        } else {
            &self.working_dir
        };

        env.execute_command("make", &["install"], Some(install_dir))
            .await
    }

    /// Set SBOM generation
    #[must_use]
    pub fn auto_sbom(&mut self, enable: bool) -> &mut Self {
        self.auto_sbom = enable;
        self
    }

    /// Set SBOM exclusion patterns
    #[must_use]
    pub fn sbom_excludes(&mut self, patterns: Vec<String>) -> &mut Self {
        self.sbom_excludes = patterns;
        self
    }

    /// Get SBOM configuration
    #[must_use]
    pub fn sbom_config(&self) -> (bool, &[String]) {
        (self.auto_sbom, &self.sbom_excludes)
    }

    /// Extract downloaded archives
    ///
    /// # Errors
    ///
    /// Returns an error if any archive extraction fails.
    pub async fn extract_downloads(&self) -> Result<(), Error> {
        for path in self.downloads.values() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                match ext {
                    "gz" | "tgz" => {
                        self.extract_tar_gz(path).await?;
                    }
                    "bz2" => {
                        self.extract_tar_bz2(path).await?;
                    }
                    "xz" => {
                        self.extract_tar_xz(path).await?;
                    }
                    "zip" => {
                        self.extract_zip(path).await?;
                    }
                    _ => {
                        // Unknown format, skip extraction
                    }
                }
            }
        }
        Ok(())
    }

    /// Extract tar.gz archive
    ///
    /// # Errors
    ///
    /// Returns an error if the tar command fails.
    async fn extract_tar_gz(&self, path: &Path) -> Result<(), Error> {
        let output = tokio::process::Command::new("tar")
            .args(["-xzf", &path.display().to_string()])
            .current_dir(&self.working_dir)
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::ExtractionFailed {
                message: format!(
                    "Failed to extract {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Extract tar.bz2 archive
    ///
    /// # Errors
    ///
    /// Returns an error if the tar command fails.
    async fn extract_tar_bz2(&self, path: &Path) -> Result<(), Error> {
        let output = tokio::process::Command::new("tar")
            .args(["-xjf", &path.display().to_string()])
            .current_dir(&self.working_dir)
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::ExtractionFailed {
                message: format!(
                    "Failed to extract {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Extract tar.xz archive
    ///
    /// # Errors
    ///
    /// Returns an error if the tar command fails.
    async fn extract_tar_xz(&self, path: &Path) -> Result<(), Error> {
        let output = tokio::process::Command::new("tar")
            .args(["-xJf", &path.display().to_string()])
            .current_dir(&self.working_dir)
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::ExtractionFailed {
                message: format!(
                    "Failed to extract {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Extract zip archive
    ///
    /// # Errors
    ///
    /// Returns an error if the unzip command fails.
    async fn extract_zip(&self, path: &Path) -> Result<(), Error> {
        let output = tokio::process::Command::new("unzip")
            .args(["-q", &path.display().to_string()])
            .current_dir(&self.working_dir)
            .output()
            .await?;

        if !output.status.success() {
            return Err(BuildError::ExtractionFailed {
                message: format!(
                    "Failed to extract {}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_builder_api_creation() {
        let temp = tempdir().unwrap();
        let api = BuilderApi::new(temp.path().to_path_buf()).unwrap();

        assert!(!api.allow_network);
        assert!(api.auto_sbom);
        assert!(!api.sbom_excludes.is_empty());
    }

    #[test]
    fn test_builder_api_configuration() {
        let temp = tempdir().unwrap();
        let mut api = BuilderApi::new(temp.path().to_path_buf()).unwrap();

        let _ = api
            .allow_network(true)
            .auto_sbom(false)
            .sbom_excludes(vec!["*.test".to_string()]);

        assert!(api.allow_network);

        let (sbom_enabled, excludes) = api.sbom_config();
        assert!(!sbom_enabled);
        assert_eq!(excludes, &["*.test"]);
    }
}
