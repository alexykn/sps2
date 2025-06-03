//! Store operation integration tests

use super::common::TestEnvironment;
use sps2_install::{validate_sp_file, PackageFormat};
use sps2_store::extract_package;
use tempfile::tempdir;
use tokio::fs;

/// Test package extraction using Store API - realistic package structure
#[tokio::test]
async fn test_store_package_extraction() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;

    // Create realistic test package structure
    let package_dir = temp.path().join("package");
    fs::create_dir_all(&package_dir).await?;

    let manifest_content = r#"
[package]
name = "extraction-test"
version = "1.2.3"
revision = 1
arch = "arm64"
description = "Package for extraction testing"
license = "MIT"

[dependencies]
runtime = ["zlib>=1.2.0"]
build = ["gcc>=9.0.0"]
"#;
    fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

    // Create comprehensive file structure
    let files_dir = package_dir.join("files");
    fs::create_dir_all(&files_dir.join("bin")).await?;
    fs::create_dir_all(&files_dir.join("lib")).await?;
    fs::create_dir_all(&files_dir.join("share").join("doc")).await?;

    // Create files with different content types
    let test_content = "#!/bin/bash\necho 'Hello from extraction test'\nexit 0\n";
    fs::write(files_dir.join("bin").join("test"), test_content).await?;

    let binary_content = vec![0u8, 1, 2, 3, 4, 5, 255, 254, 253]; // Binary data
    fs::write(files_dir.join("bin").join("binary"), &binary_content).await?;

    fs::write(
        files_dir.join("lib").join("libtest.so"),
        "fake library content for testing",
    )
    .await?;

    fs::write(
        files_dir.join("share").join("doc").join("README.md"),
        "# Extraction Test\n\nThis is a test package for extraction.\n",
    )
    .await?;

    // Create package using Store API (creates plain tar)
    let sp_file = temp.path().join("extraction-test-1.2.3-1.arm64.sp");
    sps2_store::create_package(&package_dir, &sp_file).await?;

    // Extract using Store API
    let extract_dir = temp.path().join("extracted");
    extract_package(&sp_file, &extract_dir).await?;

    // Verify extraction results
    assert!(extract_dir.join("manifest.toml").exists());
    assert!(extract_dir.join("files").join("bin").join("test").exists());
    assert!(extract_dir.join("files").join("bin").join("binary").exists());
    assert!(extract_dir
        .join("files")
        .join("lib")
        .join("libtest.so")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("share")
        .join("doc")
        .join("README.md")
        .exists());

    // Verify content integrity
    let extracted_content =
        fs::read_to_string(extract_dir.join("files").join("bin").join("test")).await?;
    assert_eq!(extracted_content, test_content);

    let extracted_binary =
        fs::read(extract_dir.join("files").join("bin").join("binary")).await?;
    assert_eq!(extracted_binary, binary_content);

    Ok(())
}

/// Test package extraction with various compression levels using Store API
#[tokio::test]
async fn test_extraction_compression_levels() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;

    // Create package with compressible content
    let package_dir = temp.path().join("package");
    fs::create_dir_all(&package_dir).await?;

    let manifest_content = r#"
[package]
name = "compression-test"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "Package for compression testing"
license = "MIT"
"#;
    fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

    let files_dir = package_dir.join("files");
    fs::create_dir_all(&files_dir.join("data")).await?;

    // Create highly compressible content
    let repetitive_content = "This is repetitive content for compression testing.\n".repeat(1000);
    fs::write(
        files_dir.join("data").join("repetitive.txt"),
        &repetitive_content,
    )
    .await?;

    // Create less compressible content
    let random_content = (0..5000).map(|i| (i % 256) as u8).collect::<Vec<u8>>();
    fs::write(files_dir.join("data").join("random.bin"), &random_content).await?;

    // Create package using Store API (creates plain tar)
    let sp_file = temp.path().join("compression-test-1.0.0-1.arm64.sp");
    sps2_store::create_package(&package_dir, &sp_file).await?;

    // Validate package
    let validation = validate_sp_file(&sp_file, None).await?;
    assert!(validation.is_valid);
    assert_eq!(validation.format, PackageFormat::PlainTar);

    // Extract using Store API
    let extract_dir = temp.path().join("extracted");
    extract_package(&sp_file, &extract_dir).await?;

    // Verify content integrity after compression/decompression
    let extracted_text =
        fs::read_to_string(extract_dir.join("files").join("data").join("repetitive.txt")).await?;
    assert_eq!(extracted_text, repetitive_content);

    let extracted_binary =
        fs::read(extract_dir.join("files").join("data").join("random.bin")).await?;
    assert_eq!(extracted_binary, random_content);

    Ok(())
}

/// Test package validation pipeline using Store API
#[tokio::test]
async fn test_package_validation_pipeline() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;

    // Create comprehensive test package
    let package_dir = temp.path().join("package");
    fs::create_dir_all(&package_dir).await?;

    let manifest_content = r#"
[package]
name = "validation-test"
version = "2.1.0"
revision = 1
arch = "arm64"
description = "Package for validation testing"
license = "Apache-2.0"
homepage = "https://example.com/validation-test"

[dependencies]
runtime = ["libssl>=1.1.0", "zlib>=1.2.0"]
build = ["make>=4.0", "gcc>=9.0.0"]
"#;
    fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

    // Add SBOM files
    fs::write(
        package_dir.join("sbom.spdx.json"),
        r#"{"spdxVersion": "SPDX-2.3", "name": "validation-test", "packages": []}"#,
    )
    .await?;

    // Create comprehensive file structure
    let files_dir = package_dir.join("files");
    fs::create_dir_all(&files_dir.join("bin")).await?;
    fs::create_dir_all(&files_dir.join("lib")).await?;
    fs::create_dir_all(&files_dir.join("share").join("doc")).await?;
    fs::create_dir_all(&files_dir.join("etc")).await?;

    // Create various file types
    fs::write(
        files_dir.join("bin").join("main"),
        "#!/bin/bash\necho 'validation-test main program'\n",
    )
    .await?;

    fs::write(
        files_dir.join("lib").join("libvalidation.so"),
        "fake library content for validation testing",
    )
    .await?;

    fs::write(
        files_dir.join("share").join("doc").join("README.md"),
        "# validation-test 2.1.0\n\nComprehensive validation testing package.\n",
    )
    .await?;

    fs::write(
        files_dir.join("etc").join("config.conf"),
        "# Configuration file\nversion=2.1.0\nmode=test\n",
    )
    .await?;

    // Create package using Store API
    let sp_file = temp.path().join("validation-test-2.1.0-1.arm64.sp");
    sps2_store::create_package(&package_dir, &sp_file).await?;

    // Validate the package
    let validation = validate_sp_file(&sp_file, None).await?;
    assert!(validation.is_valid);
    assert!(validation.file_count > 6); // manifest + sbom + at least 4 files
    assert!(validation.extracted_size > 0);
    assert!(validation.manifest.is_some());

    // Extract using Store API and verify full pipeline
    let extract_dir = temp.path().join("extracted");
    extract_package(&sp_file, &extract_dir).await?;

    // Verify all expected files exist
    assert!(extract_dir.join("manifest.toml").exists());
    assert!(extract_dir.join("sbom.spdx.json").exists());
    assert!(extract_dir.join("files").join("bin").join("main").exists());
    assert!(extract_dir
        .join("files")
        .join("lib")
        .join("libvalidation.so")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("share")
        .join("doc")
        .join("README.md")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("etc")
        .join("config.conf")
        .exists());

    // Verify content integrity
    let manifest_text = fs::read_to_string(extract_dir.join("manifest.toml")).await?;
    assert!(manifest_text.contains("validation-test"));
    assert!(manifest_text.contains("2.1.0"));
    assert!(manifest_text.contains("libssl>=1.1.0"));

    let main_content =
        fs::read_to_string(extract_dir.join("files").join("bin").join("main")).await?;
    assert!(main_content.contains("validation-test main program"));

    Ok(())
}

/// Test complete Store API round-trip: create → validate → extract
#[tokio::test]
async fn test_store_roundtrip_integration() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;

    // Create comprehensive package structure for round-trip testing
    let package_dir = temp.path().join("source");
    fs::create_dir_all(&package_dir).await?;

    // Create comprehensive package structure
    let manifest_content = r#"
[package]
name = "roundtrip-test"
version = "3.2.1"
revision = 2
arch = "arm64"
description = "End-to-end Store API test package"
homepage = "https://example.com/roundtrip-test"
license = "MIT"

[dependencies]
runtime = [
    "zlib>=1.2.0",
    "openssl~=3.0.0"
]
build = [
    "gcc>=9.0.0",
    "make>=4.0"
]
"#;
    fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

    // Add SBOM files
    fs::write(
        package_dir.join("sbom.spdx.json"),
        r#"{"spdxVersion": "SPDX-2.3", "name": "roundtrip-test", "packages": []}"#,
    )
    .await?;

    fs::write(
        package_dir.join("sbom.cdx.json"),
        r#"{"bomFormat": "CycloneDX", "specVersion": "1.6", "components": []}"#,
    )
    .await?;

    // Create realistic file structure
    let files_dir = package_dir.join("files");
    fs::create_dir_all(&files_dir.join("bin")).await?;
    fs::create_dir_all(&files_dir.join("lib")).await?;
    fs::create_dir_all(&files_dir.join("share").join("man")).await?;

    // Create diverse file types
    fs::write(
        files_dir.join("bin").join("roundtrip"),
        "#!/bin/bash\necho 'roundtrip-test successful'\nexit 0\n",
    )
    .await?;

    fs::write(
        files_dir.join("lib").join("libroundtrip.so"),
        "FAKE_LIBRARY_CONTENT_FOR_ROUNDTRIP_TESTING",
    )
    .await?;

    fs::write(
        files_dir.join("share").join("man").join("roundtrip.1"),
        ".TH ROUNDTRIP 1\n.SH NAME\nroundtrip - test program\n",
    )
    .await?;

    // Step 1: Create package using Store API (creates plain tar)
    let sp_file = temp.path().join("roundtrip-test-3.2.1-2.arm64.sp");
    sps2_store::create_package(&package_dir, &sp_file).await?;

    // Step 2: Validate package (simulating install validation)
    let validation = validate_sp_file(&sp_file, None).await?;
    assert!(validation.is_valid);
    assert!(validation.file_count >= 7); // manifest + 2 sboms + at least 4 files
    assert!(validation.extracted_size > 0);
    assert!(validation.manifest.is_some());

    // Parse and verify manifest content
    let manifest_text = validation.manifest.as_ref().unwrap();
    assert!(manifest_text.contains("roundtrip-test"));
    assert!(manifest_text.contains("3.2.1"));
    assert!(manifest_text.contains("zlib>=1.2.0"));
    assert!(manifest_text.contains("openssl~=3.0.0"));

    // Step 3: Extract package using Store API
    let extract_dir = temp.path().join("extracted");
    extract_package(&sp_file, &extract_dir).await?;

    // Verify extraction structure
    assert!(extract_dir.join("manifest.toml").exists());
    assert!(extract_dir.join("sbom.spdx.json").exists());
    assert!(extract_dir.join("sbom.cdx.json").exists());
    assert!(extract_dir
        .join("files")
        .join("bin")
        .join("roundtrip")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("lib")
        .join("libroundtrip.so")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("share")
        .join("man")
        .join("roundtrip.1")
        .exists());

    // Step 4: Verify content integrity (byte-for-byte comparison)
    let extracted_manifest = fs::read_to_string(extract_dir.join("manifest.toml")).await?;
    let original_manifest = fs::read_to_string(package_dir.join("manifest.toml")).await?;
    assert_eq!(extracted_manifest, original_manifest);

    let extracted_binary =
        fs::read_to_string(extract_dir.join("files").join("bin").join("roundtrip")).await?;
    let original_binary = fs::read_to_string(files_dir.join("bin").join("roundtrip")).await?;
    assert_eq!(extracted_binary, original_binary);

    let extracted_lib = fs::read_to_string(
        extract_dir
            .join("files")
            .join("lib")
            .join("libroundtrip.so"),
    )
    .await?;
    let original_lib =
        fs::read_to_string(files_dir.join("lib").join("libroundtrip.so")).await?;
    assert_eq!(extracted_lib, original_lib);

    let extracted_man = fs::read_to_string(
        extract_dir
            .join("files")
            .join("share")
            .join("man")
            .join("roundtrip.1"),
    )
    .await?;
    let original_man =
        fs::read_to_string(files_dir.join("share").join("man").join("roundtrip.1")).await?;
    assert_eq!(extracted_man, original_man);

    // Step 5: Verify SBOM files
    let extracted_spdx = fs::read_to_string(extract_dir.join("sbom.spdx.json")).await?;
    let original_spdx = fs::read_to_string(package_dir.join("sbom.spdx.json")).await?;
    assert_eq!(extracted_spdx, original_spdx);

    Ok(())
}

/// Test Store API with different package sizes and content types
#[tokio::test]
async fn test_store_package_sizes() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;

    // Create package with diverse content to test Store API robustness
    let package_dir = temp.path().join("package");
    fs::create_dir_all(&package_dir).await?;

    let manifest_content = r#"
[package]
name = "size-test"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "Package for testing different content sizes"
license = "MIT"

[dependencies]
runtime = ["base-runtime"]
build = ["build-tools"]
"#;
    fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

    // Create files with varying content characteristics
    let files_dir = package_dir.join("files");
    fs::create_dir_all(&files_dir.join("data")).await?;
    fs::create_dir_all(&files_dir.join("bin")).await?;
    fs::create_dir_all(&files_dir.join("config")).await?;

    // Large text file with repetitive content
    let repetitive_content = "This line repeats many times for size testing.\n".repeat(2000);
    fs::write(
        files_dir.join("data").join("large.txt"),
        &repetitive_content,
    )
    .await?;

    // Binary-like content that doesn't compress well
    let binary_content = (0..5000).map(|i| (i % 256) as u8).collect::<Vec<u8>>();
    fs::write(files_dir.join("data").join("binary.dat"), &binary_content).await?;

    // Many small files
    for i in 0..25 {
        fs::write(
            files_dir
                .join("config")
                .join(format!("config{:03}.conf", i)),
            format!("# Config file {}\nvalue={i}\nname=test-{i}\n", i),
        )
        .await?;
    }

    // Executable file
    fs::write(
        files_dir.join("bin").join("size-test"),
        "#!/bin/bash\necho 'Size test program'\necho 'Testing Store API with various content sizes'\n",
    ).await?;

    // Create package using Store API
    let sp_file = temp.path().join("size-test-1.0.0-1.arm64.sp");
    sps2_store::create_package(&package_dir, &sp_file).await?;

    // Validate the package
    let validation = validate_sp_file(&sp_file, None).await?;
    assert!(validation.is_valid);
    assert!(validation.file_count > 25); // manifest + many files
    assert!(validation.extracted_size > 50000); // Should be reasonably large

    // Extract using Store API
    let extract_dir = temp.path().join("extracted");
    extract_package(&sp_file, &extract_dir).await?;

    // Verify all content types were extracted correctly
    assert!(extract_dir.join("manifest.toml").exists());
    assert!(extract_dir
        .join("files")
        .join("data")
        .join("large.txt")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("data")
        .join("binary.dat")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("bin")
        .join("size-test")
        .exists());

    // Verify some config files exist
    assert!(extract_dir
        .join("files")
        .join("config")
        .join("config000.conf")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("config")
        .join("config024.conf")
        .exists());

    // Verify content integrity for different types
    let extracted_text =
        fs::read_to_string(extract_dir.join("files").join("data").join("large.txt")).await?;
    assert_eq!(extracted_text, repetitive_content);

    let extracted_binary =
        fs::read(extract_dir.join("files").join("data").join("binary.dat")).await?;
    assert_eq!(extracted_binary, binary_content);

    let config_content = fs::read_to_string(
        extract_dir
            .join("files")
            .join("config")
            .join("config010.conf"),
    )
    .await?;
    assert!(config_content.contains("Config file 10"));
    assert!(config_content.contains("value=10"));

    Ok(())
}

/// Test Store API edge cases and error handling
#[tokio::test]
async fn test_store_api_edge_cases() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;

    // Test empty package
    let empty_dir = temp.path().join("empty");
    fs::create_dir_all(&empty_dir).await?;

    // Empty package should fail to validate (no manifest)
    let empty_sp = temp.path().join("empty.sp");
    let result = sps2_store::create_package(&empty_dir, &empty_sp).await;
    assert!(result.is_err()); // Should fail without manifest

    // Test package with only manifest
    let minimal_dir = temp.path().join("minimal");
    fs::create_dir_all(&minimal_dir).await?;
    fs::write(
        minimal_dir.join("manifest.toml"),
        r#"
[package]
name = "minimal"
version = "1.0.0"
revision = 1
arch = "arm64"
description = "Minimal package"
license = "MIT"
"#,
    )
    .await?;

    let minimal_sp = temp.path().join("minimal.sp");
    sps2_store::create_package(&minimal_dir, &minimal_sp).await?;

    let validation = validate_sp_file(&minimal_sp, None).await?;
    assert!(validation.is_valid);
    assert!(validation.file_count >= 1); // At least manifest

    Ok(())
}

/// Test Store API with various package structures and formats
#[tokio::test]
async fn test_store_package_formats() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;

    // Test different package structures that Store API should handle
    let package_dir = temp.path().join("format_test");
    fs::create_dir_all(&package_dir).await?;

    // Standard manifest
    let manifest_content = r#"
[package]
name = "format-test"
version = "2.0.0"
revision = 1
arch = "arm64"
description = "Package for format testing"
homepage = "https://example.com/format-test"
license = "Apache-2.0"

[dependencies]
runtime = ["base-system>=1.0.0"]
build = ["build-essential"]
"#;
    fs::write(package_dir.join("manifest.toml"), manifest_content).await?;

    // Standard files structure
    let files_dir = package_dir.join("files");
    fs::create_dir_all(&files_dir.join("bin")).await?;
    fs::create_dir_all(&files_dir.join("lib")).await?;
    fs::create_dir_all(&files_dir.join("include")).await?;
    fs::create_dir_all(&files_dir.join("share").join("doc")).await?;

    // Create various file types
    fs::write(
        files_dir.join("bin").join("format-test"),
        "#!/bin/bash\necho 'Format test application'\necho 'Testing Store API format handling'\n",
    ).await?;

    fs::write(
        files_dir.join("lib").join("libformat.so"),
        "ELF_FAKE_BINARY_CONTENT_FOR_LIBRARY_TESTING",
    )
    .await?;

    fs::write(
        files_dir.join("include").join("format.h"),
        "#ifndef FORMAT_H\n#define FORMAT_H\n\nvoid format_test(void);\n\n#endif\n",
    )
    .await?;

    fs::write(
        files_dir.join("share").join("doc").join("README.txt"),
        "Format Test Package\n==================\n\nThis package tests Store API format handling.\n",
    ).await?;

    // Create package using Store API (creates plain tar)
    let sp_file = temp.path().join("format-test-2.0.0-1.arm64.sp");
    sps2_store::create_package(&package_dir, &sp_file).await?;

    // Verify package creation and format detection
    let validation = validate_sp_file(&sp_file, None).await?;
    assert!(validation.is_valid);
    assert_eq!(validation.format, PackageFormat::PlainTar);
    assert!(validation.file_count >= 6); // manifest + dirs + files

    // Extract using Store API
    let extract_dir = temp.path().join("format_extracted");
    extract_package(&sp_file, &extract_dir).await?;

    // Verify all file types were preserved
    assert!(extract_dir.join("manifest.toml").exists());
    assert!(extract_dir
        .join("files")
        .join("bin")
        .join("format-test")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("lib")
        .join("libformat.so")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("include")
        .join("format.h")
        .exists());
    assert!(extract_dir
        .join("files")
        .join("share")
        .join("doc")
        .join("README.txt")
        .exists());

    // Verify content preservation for different file types
    let script_content =
        fs::read_to_string(extract_dir.join("files").join("bin").join("format-test")).await?;
    assert!(script_content.contains("Format test application"));
    assert!(script_content.contains("Testing Store API format handling"));

    let header_content =
        fs::read_to_string(extract_dir.join("files").join("include").join("format.h")).await?;
    assert!(header_content.contains("#ifndef FORMAT_H"));
    assert!(header_content.contains("void format_test(void);"));

    let doc_content = fs::read_to_string(
        extract_dir
            .join("files")
            .join("share")
            .join("doc")
            .join("README.txt"),
    )
    .await?;
    assert!(doc_content.contains("Format Test Package"));
    assert!(doc_content.contains("Store API format handling"));

    let lib_content =
        fs::read_to_string(extract_dir.join("files").join("lib").join("libformat.so")).await?;
    assert!(lib_content.contains("ELF_FAKE_BINARY_CONTENT"));

    Ok(())
}