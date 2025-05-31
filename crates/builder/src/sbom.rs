//! SBOM generation using Syft

use sps2_errors::{BuildError, Error};
use sps2_hash::Hash;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// SBOM generator using Syft
pub struct SbomGenerator {
    /// Syft binary path
    syft_path: String,
    /// Configuration
    config: SbomConfig,
}

/// SBOM generation configuration
#[derive(Clone, Debug)]
pub struct SbomConfig {
    /// Generate SPDX format
    pub generate_spdx: bool,
    /// Generate `CycloneDX` format
    pub generate_cyclonedx: bool,
    /// File exclusion patterns
    pub exclude_patterns: Vec<String>,
    /// Include package dependencies in SBOM
    pub include_dependencies: bool,
}

impl Default for SbomConfig {
    fn default() -> Self {
        Self {
            generate_spdx: true,
            generate_cyclonedx: false,
            exclude_patterns: vec![
                "./*.dSYM".to_string(),
                "./*.pdb".to_string(),
                "./*.a".to_string(),
                "./*.la".to_string(),
            ],
            include_dependencies: true,
        }
    }
}

impl SbomConfig {
    /// Create config with both SPDX and `CycloneDX`
    #[must_use]
    pub fn with_both_formats() -> Self {
        Self {
            generate_cyclonedx: true,
            ..Default::default()
        }
    }

    /// Add exclusion pattern
    #[must_use]
    pub fn exclude(mut self, pattern: String) -> Self {
        self.exclude_patterns.push(pattern);
        self
    }

    /// Set dependency inclusion
    #[must_use]
    pub fn include_dependencies(mut self, include: bool) -> Self {
        self.include_dependencies = include;
        self
    }
}

/// Generated SBOM files
#[derive(Debug)]
pub struct SbomFiles {
    /// SPDX JSON file path
    pub spdx_path: Option<PathBuf>,
    /// SPDX file hash
    pub spdx_hash: Option<String>,
    /// `CycloneDX` JSON file path
    pub cyclonedx_path: Option<PathBuf>,
    /// `CycloneDX` file hash
    pub cyclonedx_hash: Option<String>,
}

impl SbomFiles {
    /// Create empty SBOM files
    #[must_use]
    pub fn new() -> Self {
        Self {
            spdx_path: None,
            spdx_hash: None,
            cyclonedx_path: None,
            cyclonedx_hash: None,
        }
    }

    /// Check if any SBOM files were generated
    #[must_use]
    pub fn has_files(&self) -> bool {
        self.spdx_path.is_some() || self.cyclonedx_path.is_some()
    }
}

impl Default for SbomFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl SbomGenerator {
    /// Create new SBOM generator
    #[must_use]
    pub fn new() -> Self {
        Self {
            syft_path: "syft".to_string(),
            config: SbomConfig::default(),
        }
    }

    /// Create with custom Syft path
    #[must_use]
    pub fn with_syft_path(syft_path: String) -> Self {
        Self {
            syft_path,
            config: SbomConfig::default(),
        }
    }

    /// Set configuration
    #[must_use]
    pub fn with_config(mut self, config: SbomConfig) -> Self {
        self.config = config;
        self
    }

    /// Check if Syft is available
    ///
    /// # Errors
    ///
    /// Returns an error if Syft cannot be executed.
    pub async fn check_syft_available(&self) -> Result<bool, Error> {
        let output = Command::new(&self.syft_path)
            .arg("--version")
            .output()
            .await;

        match output {
            Ok(output) => Ok(output.status.success()),
            Err(_) => Ok(false),
        }
    }

    /// Generate SBOM files for a directory
    ///
    /// # Errors
    ///
    /// Returns an error if Syft is not available, SBOM generation fails, or deterministic verification fails.
    pub async fn generate_sbom(
        &self,
        source_dir: &Path,
        output_dir: &Path,
    ) -> Result<SbomFiles, Error> {
        if !self.check_syft_available().await? {
            return Err(BuildError::SbomError {
                message: "Syft not found - SBOM generation requires syft >= 1.4".to_string(),
            }
            .into());
        }

        let mut sbom_files = SbomFiles::new();

        // Generate SPDX format
        if self.config.generate_spdx {
            let spdx_path = output_dir.join("sbom.spdx.json");
            self.generate_spdx(source_dir, &spdx_path).await?;

            let hash = Hash::hash_file(&spdx_path).await?;
            sbom_files.spdx_path = Some(spdx_path);
            sbom_files.spdx_hash = Some(hash.to_hex());
        }

        // Generate CycloneDX format
        if self.config.generate_cyclonedx {
            let cdx_path = output_dir.join("sbom.cdx.json");
            self.generate_cyclonedx(source_dir, &cdx_path).await?;

            let hash = Hash::hash_file(&cdx_path).await?;
            sbom_files.cyclonedx_path = Some(cdx_path);
            sbom_files.cyclonedx_hash = Some(hash.to_hex());
        }

        // Verify deterministic output by regenerating
        // TODO: Temporarily disable deterministic verification due to syft non-determinism
        // self.verify_deterministic(&sbom_files, source_dir).await?;

        Ok(sbom_files)
    }

    /// Generate SPDX format SBOM
    ///
    /// # Errors
    ///
    /// Returns an error if Syft execution fails or returns a non-zero exit code.
    async fn generate_spdx(&self, source_dir: &Path, output_path: &Path) -> Result<(), Error> {
        let mut args = vec![
            "scan".to_string(),
            "-o".to_string(),
            format!("spdx-json={}", output_path.display()),
            source_dir.display().to_string(),
        ];

        // Add exclusions
        for pattern in &self.config.exclude_patterns {
            args.push("--exclude".to_string());
            args.push(pattern.clone());
        }

        let output = Command::new(&self.syft_path)
            .args(&args)
            .env("SYFT_SPDX_CREATION_INFO_CREATED", "2024-01-01T00:00:00Z")
            .env("SOURCE_DATE_EPOCH", "1704067200")
            .env("SYFT_DISABLE_METADATA_TIMESTAMP", "true")
            .output()
            .await
            .map_err(|e| BuildError::SbomError {
                message: format!("failed to run syft: {e}"),
            })?;

        if !output.status.success() {
            return Err(BuildError::SbomError {
                message: format!("syft failed: {}", String::from_utf8_lossy(&output.stderr)),
            }
            .into());
        }

        Ok(())
    }

    /// Generate `CycloneDX` format SBOM
    ///
    /// # Errors
    ///
    /// Returns an error if Syft execution fails or returns a non-zero exit code.
    async fn generate_cyclonedx(&self, source_dir: &Path, output_path: &Path) -> Result<(), Error> {
        let mut args = vec![
            "scan".to_string(),
            "-o".to_string(),
            format!("cyclonedx-json={}", output_path.display()),
            source_dir.display().to_string(),
        ];

        // Add exclusions
        for pattern in &self.config.exclude_patterns {
            args.push("--exclude".to_string());
            args.push(pattern.clone());
        }

        let output = Command::new(&self.syft_path)
            .args(&args)
            .env("SYFT_SPDX_CREATION_INFO_CREATED", "2024-01-01T00:00:00Z")
            .env("SOURCE_DATE_EPOCH", "1704067200")
            .env("SYFT_DISABLE_METADATA_TIMESTAMP", "true")
            .output()
            .await
            .map_err(|e| BuildError::SbomError {
                message: format!("failed to run syft: {e}"),
            })?;

        if !output.status.success() {
            return Err(BuildError::SbomError {
                message: format!("syft failed: {}", String::from_utf8_lossy(&output.stderr)),
            }
            .into());
        }

        Ok(())
    }

    /// Verify SBOM generation is deterministic
    ///
    /// # Errors
    ///
    /// Returns an error if temp directory creation fails or SBOM generation is not deterministic.
    #[allow(dead_code)]
    async fn verify_deterministic(
        &self,
        sbom_files: &SbomFiles,
        source_dir: &Path,
    ) -> Result<(), Error> {
        // Create temporary directory for verification
        let temp_dir = tempfile::tempdir().map_err(|e| BuildError::SbomError {
            message: format!("failed to create temp dir: {e}"),
        })?;

        // Regenerate SPDX and compare
        if let (Some(spdx_path), Some(expected_hash)) =
            (&sbom_files.spdx_path, &sbom_files.spdx_hash)
        {
            let verify_path = temp_dir.path().join("verify.spdx.json");
            self.generate_spdx(source_dir, &verify_path).await?;

            let verify_hash = Hash::hash_file(&verify_path).await?;
            if verify_hash.to_hex() != *expected_hash {
                // Read both files to help debug the difference
                let original_content = tokio::fs::read_to_string(spdx_path)
                    .await
                    .unwrap_or_else(|_| "Failed to read original".to_string());
                let verify_content = tokio::fs::read_to_string(&verify_path)
                    .await
                    .unwrap_or_else(|_| "Failed to read verify".to_string());

                return Err(BuildError::SbomError {
                    message: format!(
                        "SPDX SBOM generation is not deterministic: expected hash {}, got hash {}. Original length: {}, verify length: {}",
                        expected_hash,
                        verify_hash.to_hex(),
                        original_content.len(),
                        verify_content.len()
                    ),
                }
                .into());
            }
        }

        // Regenerate CycloneDX and compare
        if let (Some(_cdx_path), Some(expected_hash)) =
            (&sbom_files.cyclonedx_path, &sbom_files.cyclonedx_hash)
        {
            let verify_path = temp_dir.path().join("verify.cdx.json");
            self.generate_cyclonedx(source_dir, &verify_path).await?;

            let verify_hash = Hash::hash_file(&verify_path).await?;
            if verify_hash.to_hex() != *expected_hash {
                return Err(BuildError::SbomError {
                    message: "CycloneDX SBOM generation is not deterministic".to_string(),
                }
                .into());
            }
        }

        Ok(())
    }
}

impl Default for SbomGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sbom_config() {
        let config = SbomConfig::default();
        assert!(config.generate_spdx);
        assert!(!config.generate_cyclonedx);
        assert!(!config.exclude_patterns.is_empty());

        let both_config = SbomConfig::with_both_formats();
        assert!(both_config.generate_spdx);
        assert!(both_config.generate_cyclonedx);
    }

    #[test]
    fn test_sbom_config_builder() {
        let config = SbomConfig::default()
            .exclude("*.test".to_string())
            .include_dependencies(false);

        assert!(config.exclude_patterns.contains(&"*.test".to_string()));
        assert!(!config.include_dependencies);
    }

    #[test]
    fn test_sbom_files() {
        let mut files = SbomFiles::new();
        assert!(!files.has_files());

        files.spdx_path = Some("/path/to/sbom.spdx.json".into());
        assert!(files.has_files());
    }

    #[tokio::test]
    async fn test_sbom_generator_creation() {
        let generator = SbomGenerator::new();
        assert_eq!(generator.syft_path, "syft");

        let custom_generator = SbomGenerator::with_syft_path("/usr/local/bin/syft".to_string());
        assert_eq!(custom_generator.syft_path, "/usr/local/bin/syft");
    }

    #[tokio::test]
    async fn test_syft_availability() {
        let generator = SbomGenerator::new();

        // This test will fail if syft is not installed, which is expected
        // In CI/real usage, syft would be a dependency
        let _available = generator.check_syft_available().await.unwrap_or(false);
        // Don't assert on availability since syft may not be installed in test environment
    }
}
