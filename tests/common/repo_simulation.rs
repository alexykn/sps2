//! Repository simulation for testing URL resolution and download workflows
//!
//! This module provides a mock repository server that can be used for testing
//! the complete download and installation pipeline without requiring real
//! network infrastructure.

use sps2_index::{DependencyInfo, Index, VersionEntry};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use uuid::Uuid;

/// Mock repository server for testing
pub struct MockRepository {
    /// Repository index with packages
    pub index: Index,
    /// Package files mapped by URL
    pub packages: HashMap<String, Vec<u8>>,
    /// Base URL for the mock repository
    pub base_url: String,
}

impl MockRepository {
    /// Create a new mock repository
    pub fn new() -> Self {
        Self {
            index: Index::new(),
            packages: HashMap::new(),
            base_url: "https://test-repo.example.com".to_string(),
        }
    }

    /// Create a new mock repository with custom base URL
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            index: Index::new(),
            packages: HashMap::new(),
            base_url,
        }
    }

    /// Add a mock package to the repository
    pub fn add_package(
        &mut self,
        name: &str,
        version: &str,
        dependencies: Vec<&str>,
        package_data: Vec<u8>,
    ) -> String {
        let package_id = format!("{}-{}-1.arm64.sp", name, version);
        let download_url = format!("{}/packages/{}", self.base_url, package_id);
        let minisig_url = format!("{}.minisig", download_url);

        // Create version entry
        let version_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: format!("blake3_hash_{}", package_id),
            download_url: download_url.clone(),
            minisig_url,
            dependencies: DependencyInfo {
                runtime: dependencies.iter().map(|&s| s.to_string()).collect(),
                build: Vec::new(),
            },
            sbom: None,
            description: Some(format!("Mock package {}", name)),
            homepage: None,
            license: Some("MIT".to_string()),
        };

        // Add to index
        self.index
            .add_version(name.to_string(), version.to_string(), version_entry);

        // Store package data
        self.packages.insert(download_url.clone(), package_data);

        download_url
    }

    /// Create a simple test package with basic structure
    pub async fn create_test_package(
        name: &str,
        version: &str,
        dependencies: Vec<&str>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        // Create a temporary directory for package creation
        let temp_dir = std::env::temp_dir().join(format!("mock_package_{}", Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).await?;

        // Create manifest.toml
        let manifest_content = format!(
            r#"[package]
name = "{}"
version = "{}"
revision = 1
arch = "arm64"

[dependencies]
runtime = [{}]
build = []

[sbom]
spdx = "mock_spdx_hash_{}_{}"
"#,
            name,
            version,
            dependencies
                .iter()
                .map(|d| format!("\"{}\"", d))
                .collect::<Vec<_>>()
                .join(", "),
            name,
            version
        );

        let manifest_path = temp_dir.join("manifest.toml");
        fs::write(&manifest_path, manifest_content).await?;

        // Create a simple binary file
        let bin_dir = temp_dir.join("bin");
        fs::create_dir_all(&bin_dir).await?;
        let bin_path = bin_dir.join(name);
        fs::write(
            &bin_path,
            format!("#!/bin/sh\necho 'Mock {} v{}'\n", name, version),
        )
        .await?;

        // Create tar archive
        let tar_path = temp_dir.join("package.tar");
        let tar_output = tokio::process::Command::new("tar")
            .args([
                "--create",
                "--file",
                &tar_path.display().to_string(),
                "--directory",
                &temp_dir.display().to_string(),
                "manifest.toml",
                "bin",
            ])
            .output()
            .await?;

        if !tar_output.status.success() {
            return Err(format!(
                "tar failed: {}",
                String::from_utf8_lossy(&tar_output.stderr)
            )
            .into());
        }

        // Compress with zstd
        let sp_path = temp_dir.join("package.sp");
        let zstd_output = tokio::process::Command::new("zstd")
            .args([
                "--compress",
                "-o",
                &sp_path.display().to_string(),
                &tar_path.display().to_string(),
            ])
            .output()
            .await?;

        if !zstd_output.status.success() {
            return Err(format!(
                "zstd failed: {}",
                String::from_utf8_lossy(&zstd_output.stderr)
            )
            .into());
        }

        // Read the compressed package
        let package_data = fs::read(&sp_path).await?;

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir).await;

        Ok(package_data)
    }

    /// Get the repository index as JSON
    pub fn get_index_json(&self) -> Result<String, sps2_errors::Error> {
        self.index.to_json()
    }

    /// Setup a complete test repository with common packages
    pub async fn setup_common_packages(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("DEBUG: Setting up common packages...");

        // Create curl with openssl dependency
        let curl_data = Self::create_test_package("curl", "8.5.0", vec!["openssl>=3.0.0"]).await?;
        self.add_package("curl", "8.5.0", vec!["openssl>=3.0.0"], curl_data);
        println!("DEBUG: Added curl 8.5.0 with openssl dependency");

        // Create openssl with no dependencies
        let openssl_data = Self::create_test_package("openssl", "3.0.12", vec![]).await?;
        self.add_package("openssl", "3.0.12", vec![], openssl_data);
        println!("DEBUG: Added openssl 3.0.12");

        // Create jq with oniguruma dependency
        let jq_data = Self::create_test_package("jq", "1.7.1", vec!["oniguruma>=6.9.8"]).await?;
        self.add_package("jq", "1.7.1", vec!["oniguruma>=6.9.8"], jq_data);
        println!("DEBUG: Added jq 1.7.1 with oniguruma dependency");

        // Create oniguruma with no dependencies
        let oniguruma_data = Self::create_test_package("oniguruma", "6.9.8", vec![]).await?;
        self.add_package("oniguruma", "6.9.8", vec![], oniguruma_data);
        println!("DEBUG: Added oniguruma 6.9.8");

        println!(
            "DEBUG: Setup complete, index contains {} packages",
            self.index.packages.len()
        );

        Ok(())
    }

    /// Save a package file to the given path for testing
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub async fn save_package_file(
        &self,
        url: &str,
        path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(data) = self.packages.get(url) {
            fs::write(path, data).await?;
            Ok(())
        } else {
            Err(format!("Package not found for URL: {}", url).into())
        }
    }

    /// Get package data by URL
    pub fn get_package_data(&self, url: &str) -> Option<&Vec<u8>> {
        self.packages.get(url)
    }
}

impl Default for MockRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_repository() {
        let mut repo = MockRepository::new();

        // Setup common packages
        repo.setup_common_packages().await.unwrap();

        // Verify index contains packages
        let index_json = repo.get_index_json().unwrap();
        assert!(index_json.contains("curl"));
        assert!(index_json.contains("openssl"));
        assert!(index_json.contains("jq"));
        assert!(index_json.contains("oniguruma"));

        // Verify package data exists
        let curl_url = format!("{}/packages/curl-8.5.0-1.arm64.sp", repo.base_url);
        assert!(repo.get_package_data(&curl_url).is_some());
    }

    #[tokio::test]
    async fn test_create_test_package() {
        let package_data =
            MockRepository::create_test_package("test-package", "1.0.0", vec!["dependency>=1.0.0"])
                .await
                .unwrap();

        // Package data should not be empty
        assert!(!package_data.is_empty());

        // Should be compressed (zstd magic number)
        assert_eq!(&package_data[0..4], b"\x28\xb5\x2f\xfd");
    }
}
