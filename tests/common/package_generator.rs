//! Test package generation utilities for comprehensive download testing
//!
//! This module provides utilities to generate realistic .sp package files
//! with various characteristics for testing different scenarios.

use sps2_hash::Hash;
use sps2_types::Version;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tokio::fs;

/// Configuration for generating test packages
#[derive(Debug, Clone)]
#[allow(dead_code)] // Test infrastructure - not all fields used yet
pub struct TestPackageConfig {
    /// Package name
    pub name: String,
    /// Package version
    pub version: Version,
    /// Target size in bytes (approximate)
    pub target_size: u64,
    /// Content pattern for the package
    pub content_pattern: ContentPattern,
    /// Whether to use zstd compression
    pub use_compression: bool,
    /// Whether to generate a signature file
    pub generate_signature: bool,
    /// Number of files to include in the package
    pub file_count: u32,
    /// Optional dependencies
    pub dependencies: Vec<(String, Version)>,
}

impl Default for TestPackageConfig {
    fn default() -> Self {
        Self {
            name: "test-package".to_string(),
            version: Version::parse("1.0.0").unwrap(),
            target_size: 1024 * 1024, // 1MB
            content_pattern: ContentPattern::Random,
            use_compression: true,
            generate_signature: false,
            file_count: 5,
            dependencies: Vec::new(),
        }
    }
}

/// Content patterns for generating test data
#[derive(Debug, Clone)]
pub enum ContentPattern {
    /// Random bytes (good for testing compression)
    Random,
    /// Repeating pattern (high compression ratio)
    Repeating(u8),
    /// Text content (realistic file content)
    Text,
    /// Binary executable-like content
    Binary,
    /// Mixed content with different compression characteristics
    Mixed,
}

/// Generated test package information
#[derive(Debug)]
#[allow(dead_code)] // Test infrastructure - not all fields used yet
pub struct GeneratedTestPackage {
    /// Path to the .sp package file
    pub package_path: PathBuf,
    /// Path to the .minisig signature file (if generated)
    pub signature_path: Option<PathBuf>,
    /// Package configuration used
    pub config: TestPackageConfig,
    /// Actual package size
    pub actual_size: u64,
    /// Package hash
    pub hash: Hash,
    /// Compression ratio achieved
    pub compression_ratio: f64,
}

/// Test package generator
pub struct TestPackageGenerator {
    temp_dir: TempDir,
}

/// Mock build result for testing
struct MockBuildResult {
    package_path: PathBuf,
}

impl TestPackageGenerator {
    /// Create a new test package generator
    ///
    /// # Errors
    ///
    /// Returns an error if the temporary directory cannot be created.
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;

        fs::create_dir_all(&temp_dir.path().join("packages")).await?;
        fs::create_dir_all(&temp_dir.path().join("build")).await?;

        Ok(Self { temp_dir })
    }

    /// Generate a test package with the given configuration
    ///
    /// # Errors
    ///
    /// Returns an error if package generation fails.
    pub async fn generate_package(
        &self,
        config: TestPackageConfig,
    ) -> Result<GeneratedTestPackage, Box<dyn std::error::Error>> {
        // Create package source directory
        let package_source = self
            .temp_dir
            .path()
            .join(format!("{}-{}", config.name, config.version));
        fs::create_dir_all(&package_source).await?;

        // Generate package content
        let uncompressed_size = self
            .generate_package_content(&package_source, &config)
            .await?;

        // Create manifest
        let manifest_content = self.create_manifest(&config);
        let manifest_path = package_source.join("manifest.toml");
        fs::write(&manifest_path, manifest_content).await?;

        // Create build recipe
        let recipe_content = self.create_build_recipe(&config);
        let recipe_path = package_source.join("build.star");
        fs::write(&recipe_path, recipe_content).await?;

        // Build the package (placeholder - would need proper BuildContext in real implementation)
        // For now, create the package manually using TAR
        let package_path = self
            .temp_dir
            .path()
            .join("packages")
            .join(format!("{}-{}.sp", config.name, config.version));

        // Create TAR package manually for testing
        self.create_tar_package(&package_source, &package_path)
            .await?;

        let build_result = MockBuildResult {
            package_path: package_path.clone(),
        };

        // Get package file info
        let package_path = build_result.package_path;
        let actual_size = fs::metadata(&package_path).await?.len();
        let hash = Hash::hash_file(&package_path).await?;

        let compression_ratio = if uncompressed_size > 0 {
            actual_size as f64 / uncompressed_size as f64
        } else {
            1.0
        };

        // Generate signature if requested
        let signature_path = if config.generate_signature {
            let sig_path = package_path.with_extension("sp.minisig");
            self.generate_fake_signature(&package_path, &sig_path)
                .await?;
            Some(sig_path)
        } else {
            None
        };

        Ok(GeneratedTestPackage {
            package_path,
            signature_path,
            config,
            actual_size,
            hash,
            compression_ratio,
        })
    }

    /// Generate multiple test packages with different characteristics
    ///
    /// # Errors
    ///
    /// Returns an error if any package generation fails.
    pub async fn generate_package_suite(
        &self,
    ) -> Result<Vec<GeneratedTestPackage>, Box<dyn std::error::Error>> {
        let mut packages = Vec::new();

        // Small package with high compression
        packages.push(
            self.generate_package(TestPackageConfig {
                name: "small-repeated".to_string(),
                version: Version::parse("1.0.0")?,
                target_size: 64 * 1024, // 64KB
                content_pattern: ContentPattern::Repeating(0xAA),
                use_compression: true,
                generate_signature: true,
                file_count: 3,
                dependencies: Vec::new(),
            })
            .await?,
        );

        // Medium package with random content
        packages.push(
            self.generate_package(TestPackageConfig {
                name: "medium-random".to_string(),
                version: Version::parse("2.0.0")?,
                target_size: 512 * 1024, // 512KB
                content_pattern: ContentPattern::Random,
                use_compression: true,
                generate_signature: false,
                file_count: 10,
                dependencies: vec![("small-repeated".to_string(), Version::parse("1.0.0")?)],
            })
            .await?,
        );

        // Large package with text content
        packages.push(
            self.generate_package(TestPackageConfig {
                name: "large-text".to_string(),
                version: Version::parse("3.0.0")?,
                target_size: 2 * 1024 * 1024, // 2MB
                content_pattern: ContentPattern::Text,
                use_compression: true,
                generate_signature: true,
                file_count: 20,
                dependencies: Vec::new(),
            })
            .await?,
        );

        // Binary package without compression
        packages.push(
            self.generate_package(TestPackageConfig {
                name: "binary-uncompressed".to_string(),
                version: Version::parse("1.5.0")?,
                target_size: 256 * 1024, // 256KB
                content_pattern: ContentPattern::Binary,
                use_compression: false,
                generate_signature: false,
                file_count: 1,
                dependencies: Vec::new(),
            })
            .await?,
        );

        // Mixed content package
        packages.push(
            self.generate_package(TestPackageConfig {
                name: "mixed-content".to_string(),
                version: Version::parse("4.0.0")?,
                target_size: 1024 * 1024, // 1MB
                content_pattern: ContentPattern::Mixed,
                use_compression: true,
                generate_signature: true,
                file_count: 15,
                dependencies: vec![
                    ("small-repeated".to_string(), Version::parse("1.0.0")?),
                    ("medium-random".to_string(), Version::parse("2.0.0")?),
                ],
            })
            .await?,
        );

        Ok(packages)
    }

    /// Generate a malformed package for error testing
    ///
    /// # Errors
    ///
    /// Returns an error if package generation fails.
    pub async fn generate_malformed_package(
        &self,
        name: &str,
        malformation_type: MalformationType,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let package_path = self.temp_dir.path().join(format!("{}.sp", name));

        match malformation_type {
            MalformationType::CorruptedHeader => {
                // Write invalid tar header
                fs::write(&package_path, b"INVALID_TAR_HEADER_DATA").await?;
            }
            MalformationType::IncompleteData => {
                // Create a valid start but truncate it
                let valid_data = self.create_minimal_valid_package().await?;
                let truncated_data = &valid_data[..valid_data.len() / 2];
                fs::write(&package_path, truncated_data).await?;
            }
            MalformationType::WrongMagic => {
                // Create a package with wrong magic bytes
                let mut data = self.create_minimal_valid_package().await?;
                if data.len() > 4 {
                    data[0..4].copy_from_slice(b"BAAD");
                }
                fs::write(&package_path, data).await?;
            }
            MalformationType::InvalidCompression => {
                // Create a package that claims to be compressed but isn't
                let uncompressed_data = b"Hello, World!".repeat(1000);
                fs::write(&package_path, uncompressed_data).await?;
            }
        }

        Ok(package_path)
    }

    /// Get the temporary directory path
    pub fn temp_dir(&self) -> &Path {
        self.temp_dir.path()
    }

    /// Generate package content based on the pattern
    async fn generate_package_content(
        &self,
        package_dir: &Path,
        config: &TestPackageConfig,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let mut total_size = 0u64;
        let size_per_file = config.target_size / config.file_count as u64;

        for i in 0..config.file_count {
            let file_name = format!("file{:03}.dat", i);
            let file_path = package_dir.join(&file_name);

            let content = self.generate_file_content(size_per_file, &config.content_pattern, i);
            fs::write(&file_path, &content).await?;
            total_size += content.len() as u64;
        }

        Ok(total_size)
    }

    /// Generate content for a single file
    fn generate_file_content(&self, size: u64, pattern: &ContentPattern, index: u32) -> Vec<u8> {
        let mut content = Vec::with_capacity(size as usize);

        match pattern {
            ContentPattern::Random => {
                use rand::RngCore;
                let mut rng = rand::thread_rng();
                let mut buffer = vec![0u8; size as usize];
                rng.fill_bytes(&mut buffer);
                content = buffer;
            }
            ContentPattern::Repeating(byte) => {
                content.resize(size as usize, *byte);
            }
            ContentPattern::Text => {
                let lorem_ipsum = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
                    Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. \
                    Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris \
                    nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in \
                    reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla \
                    pariatur. Excepteur sint occaecat cupidatat non proident, sunt in \
                    culpa qui officia deserunt mollit anim id est laborum.\n";

                while content.len() < size as usize {
                    let line = format!(
                        "File {} Line {}: {}",
                        index,
                        content.len() / lorem_ipsum.len(),
                        lorem_ipsum
                    );
                    content.extend_from_slice(line.as_bytes());
                }
                content.truncate(size as usize);
            }
            ContentPattern::Binary => {
                // Simulate binary executable content with headers and sections
                let header = b"\x7fELF\x02\x01\x01\x00"; // ELF header
                content.extend_from_slice(header);

                let remaining_size = size as usize - header.len();
                for i in 0..remaining_size {
                    content.push(((i + index as usize) % 256) as u8);
                }
            }
            ContentPattern::Mixed => {
                let chunk_size = size / 4;

                // Random chunk
                let mut rng = rand::thread_rng();
                let mut random_chunk = vec![0u8; chunk_size as usize];
                rand::RngCore::fill_bytes(&mut rng, &mut random_chunk);
                content.extend_from_slice(&random_chunk);

                // Repeated chunk
                let repeated_chunk = vec![0xCC; chunk_size as usize];
                content.extend_from_slice(&repeated_chunk);

                // Text chunk
                let text_line = format!("Mixed content file {} text section\n", index);
                while content.len() < (2 * chunk_size) as usize + (chunk_size as usize) {
                    content.extend_from_slice(text_line.as_bytes());
                }
                content.truncate((3 * chunk_size) as usize);

                // Binary chunk
                for i in 0..(chunk_size as usize) {
                    content.push(((i + index as usize) % 256) as u8);
                }
            }
        }

        content
    }

    /// Create a manifest for the test package
    fn create_manifest(&self, config: &TestPackageConfig) -> String {
        let mut manifest = format!(
            r#"[package]
name = "{}"
version = "{}"
description = "Test package generated for download testing"
license = "MIT"
authors = ["Test Generator <test@example.com>"]

[files]
"#,
            config.name, config.version
        );

        for i in 0..config.file_count {
            manifest.push_str(&format!("\"file{:03}.dat\" = \"bin/\"\n", i));
        }

        if !config.dependencies.is_empty() {
            manifest.push_str("\n[dependencies]\n");
            for (dep_name, dep_version) in &config.dependencies {
                manifest.push_str(&format!("\"{}\" = \"{}\"\n", dep_name, dep_version));
            }
        }

        manifest
    }

    /// Create a build recipe for the test package
    fn create_build_recipe(&self, config: &TestPackageConfig) -> String {
        format!(
            r#"# Test build recipe for {}

def build(env):
    # Copy all data files to bin directory
    env.mkdir("bin")
    for i in range({}):
        filename = "file{{:03}}.dat".format(i)
        env.copy(filename, "bin/" + filename)
    
    return {{
        "success": True,
        "artifacts": ["bin/"]
    }}
"#,
            config.name, config.file_count
        )
    }

    /// Generate a fake signature file for testing
    async fn generate_fake_signature(
        &self,
        package_path: &Path,
        signature_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let fake_signature = format!(
            "untrusted comment: signature from test key\n{}\ntrusted comment: test package {}\n{}\n",
            "RWTfM2rNL1vk5QZ1+3yZLBNhQ2GBdh7eRNOv1NrNq3JpOF7dXeqTgR8h",
            package_path.file_name().unwrap_or_default().to_string_lossy(),
            "MTKnQLh5JTgPNPqH5VBFe9nMGVKqvnJ5QzJvQd9vKmJdKgP5fPTQN8Rq"
        );

        fs::write(signature_path, fake_signature).await?;
        Ok(())
    }

    /// Create a minimal valid package for malformation testing
    async fn create_minimal_valid_package(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut buffer = Vec::new();
        {
            let mut tar = tar::Builder::new(&mut buffer);

            let mut header = tar::Header::new_gnu();
            header.set_path("test.txt")?;
            header.set_size(11);
            header.set_mode(0o644);
            header.set_cksum();

            tar.append(&header, "Hello World".as_bytes())?;
            tar.finish()?;
        }

        Ok(buffer)
    }

    /// Create a TAR package manually for testing
    async fn create_tar_package(
        &self,
        source_dir: &Path,
        dest_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        tokio::task::spawn_blocking({
            let source_dir = source_dir.to_path_buf();
            let dest_path = dest_path.to_path_buf();
            move || -> std::io::Result<()> {
                use std::fs::File;
                use tar::Builder;

                let file = File::create(&dest_path)?;
                let mut tar_builder = Builder::new(file);
                tar_builder.follow_symlinks(false);

                // Add directory contents to tar
                add_dir_to_tar(&mut tar_builder, &source_dir, Path::new(""))?;
                tar_builder.finish()?;
                Ok(())
            }
        })
        .await
        .unwrap()?;
        Ok(())
    }
}

/// Helper function for adding directory to tar
fn add_dir_to_tar<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    src: &Path,
    prefix: &Path,
) -> std::io::Result<()> {
    use std::fs;

    let entries = fs::read_dir(src)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let tar_path = prefix.join(&name);

        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            builder.append_dir(&tar_path, &path)?;
            add_dir_to_tar(builder, &path, &tar_path)?;
        } else if metadata.is_file() {
            let mut file = fs::File::open(&path)?;
            builder.append_file(&tar_path, &mut file)?;
        }
    }

    Ok(())
}

/// Types of malformation for error testing
#[derive(Debug, Clone)]
#[allow(dead_code)] // Test infrastructure - not all variants used yet
pub enum MalformationType {
    /// Corrupted tar header
    CorruptedHeader,
    /// Incomplete data (truncated file)
    IncompleteData,
    /// Wrong magic bytes
    WrongMagic,
    /// Invalid compression format
    InvalidCompression,
}

/// Package suite generator for batch testing
#[allow(dead_code)] // Test infrastructure - not used yet
pub struct PackageSuiteGenerator;

impl PackageSuiteGenerator {
    /// Generate a comprehensive suite of test packages for different scenarios
    ///
    /// # Errors
    ///
    /// Returns an error if package generation fails.
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub async fn generate_comprehensive_suite(
    ) -> Result<Vec<GeneratedTestPackage>, Box<dyn std::error::Error>> {
        let generator = TestPackageGenerator::new().await?;

        let mut packages = Vec::new();

        // Small packages (< 100KB)
        for (i, pattern) in [
            ContentPattern::Random,
            ContentPattern::Repeating(0xFF),
            ContentPattern::Text,
        ]
        .iter()
        .enumerate()
        {
            packages.push(
                generator
                    .generate_package(TestPackageConfig {
                        name: format!("small-{}", i),
                        version: Version::parse("1.0.0")?,
                        target_size: 64 * 1024,
                        content_pattern: pattern.clone(),
                        use_compression: true,
                        generate_signature: i % 2 == 0,
                        file_count: 3,
                        dependencies: Vec::new(),
                    })
                    .await?,
            );
        }

        // Medium packages (100KB - 1MB)
        for (i, pattern) in [ContentPattern::Binary, ContentPattern::Mixed]
            .iter()
            .enumerate()
        {
            packages.push(
                generator
                    .generate_package(TestPackageConfig {
                        name: format!("medium-{}", i),
                        version: Version::parse("2.0.0")?,
                        target_size: 512 * 1024,
                        content_pattern: pattern.clone(),
                        use_compression: true,
                        generate_signature: true,
                        file_count: 10,
                        dependencies: Vec::new(),
                    })
                    .await?,
            );
        }

        // Large packages (> 1MB)
        packages.push(
            generator
                .generate_package(TestPackageConfig {
                    name: "large-package".to_string(),
                    version: Version::parse("3.0.0")?,
                    target_size: 5 * 1024 * 1024, // 5MB
                    content_pattern: ContentPattern::Mixed,
                    use_compression: true,
                    generate_signature: true,
                    file_count: 50,
                    dependencies: Vec::new(),
                })
                .await?,
        );

        Ok(packages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_package_generator_creation() {
        let generator = TestPackageGenerator::new().await.unwrap();
        assert!(generator.temp_dir().exists());
    }

    #[tokio::test]
    async fn test_generate_simple_package() {
        let generator = TestPackageGenerator::new().await.unwrap();
        let config = TestPackageConfig::default();

        let package = generator.generate_package(config).await.unwrap();

        assert!(package.package_path.exists());
        assert!(package.actual_size > 0);
        assert!(!package.hash.to_hex().is_empty());
    }

    #[tokio::test]
    async fn test_generate_package_with_signature() {
        let generator = TestPackageGenerator::new().await.unwrap();
        let config = TestPackageConfig {
            generate_signature: true,
            ..Default::default()
        };

        let package = generator.generate_package(config).await.unwrap();

        assert!(package.package_path.exists());
        assert!(package.signature_path.is_some());
        assert!(package.signature_path.unwrap().exists());
    }

    #[tokio::test]
    async fn test_content_pattern_generation() {
        let generator = TestPackageGenerator::new().await.unwrap();

        // Test different content patterns
        let patterns = vec![
            ContentPattern::Random,
            ContentPattern::Repeating(0xAA),
            ContentPattern::Text,
            ContentPattern::Binary,
            ContentPattern::Mixed,
        ];

        for (i, pattern) in patterns.into_iter().enumerate() {
            let config = TestPackageConfig {
                name: format!("pattern-test-{}", i),
                content_pattern: pattern,
                target_size: 1024, // Small for fast testing
                file_count: 2,
                ..Default::default()
            };

            let package = generator.generate_package(config).await.unwrap();
            assert!(package.package_path.exists());
            assert!(package.actual_size > 0);
        }
    }

    #[tokio::test]
    async fn test_malformed_package_generation() {
        let generator = TestPackageGenerator::new().await.unwrap();

        let malformed = generator
            .generate_malformed_package("corrupted", MalformationType::CorruptedHeader)
            .await
            .unwrap();

        assert!(malformed.exists());

        // Verify it's actually corrupted
        let content = fs::read(&malformed).await.unwrap();
        assert_eq!(&content[..23], b"INVALID_TAR_HEADER_DATA");
    }

    #[tokio::test]
    async fn test_package_suite_generation() {
        let generator = TestPackageGenerator::new().await.unwrap();
        let packages = generator.generate_package_suite().await.unwrap();

        assert_eq!(packages.len(), 5);

        // Verify all packages are different sizes
        let mut sizes: Vec<u64> = packages.iter().map(|p| p.actual_size).collect();
        sizes.sort_unstable();
        sizes.dedup();
        assert_eq!(sizes.len(), packages.len()); // All different sizes
    }
}
