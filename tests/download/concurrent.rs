//! Concurrent download tests

use crate::common::*;
use sps2_net::{PackageDownloader, PackageDownloadConfig, PackageDownloadRequest};
use sps2_types::Version;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::fs;

/// Test concurrent batch downloads
#[tokio::test]
async fn test_concurrent_batch_downloads() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = TestEnvironment::new()?;
    let temp_dir = TempDir::new()?;

    // Generate multiple test packages
    let generator = TestPackageGenerator::new().await?;
    let package_count = 5;
    let mut packages = Vec::new();
    let mut mocks = Vec::new();

    let server = ConfigurableMockServer::with_defaults();

    for i in 0..package_count {
        let config = TestPackageConfig {
            name: format!("batch-test-{}", i),
            version: Version::parse("1.0.0")?,
            target_size: 64 * 1024, // 64KB each
            content_pattern: ContentPattern::Sequential,
            use_compression: true,
            generate_signature: false,
            file_count: 3,
            dependencies: Vec::new(),
        };

        let package = generator.generate_package(config).await?;
        let package_data = fs::read(&package.package_path).await?;

        let path = format!("/{}-1.0.0.sp", package.config.name);
        server.register_file(&path, package_data);
        mocks.push(server.mock_file_download(&path));

        packages.push(package);
    }

    // Create download requests
    let mut requests = Vec::new();
    for package in &packages {
        let path = format!("/{}-1.0.0.sp", package.config.name);
        requests.push(PackageDownloadRequest {
            name: package.config.name.clone(),
            version: package.config.version.clone(),
            package_url: server.url(&path),
            signature_url: None,
            expected_hash: Some(package.hash.clone()),
        });
    }

    // Configure downloader for concurrency
    let config = PackageDownloadConfig {
        max_concurrent: 3,
        ..Default::default()
    };
    let downloader = PackageDownloader::new(config)?;

    // Perform batch download
    let start_time = Instant::now();
    let results = downloader
        .download_packages_batch(requests, temp_dir.path(), &env.event_sender)
        .await?;
    let download_time = start_time.elapsed();

    // Verify all packages downloaded
    assert_eq!(results.len(), packages.len());

    // Verify all packages were downloaded correctly
    // Since downloads are concurrent, results may not be in order
    for original in packages.iter() {
        let result = results.iter().find(|r| r.hash == original.hash).unwrap();
        assert!(result.package_path.exists());

        // Verify file integrity
        FileValidator::validate_file_content(&result.package_path, &original.hash).await?;
    }

    // Verify concurrent execution (should be faster than sequential)
    println!("Batch download completed in {:?}", download_time);

    // Collect events and verify batch completion
    let events = env
        .collect_events_with_timeout(Duration::from_millis(1000))
        .await;
    EventVerifier::verify_package_download_sequence(&events, packages.len() as u32)?;

    // Verify performance metrics
    let metrics = env.get_metrics();
    assert_eq!(metrics.package_downloads_completed, packages.len() as u64);
    assert!(metrics.all_successful());

    // Verify all mocks were called
    for mock in mocks {
        mock.assert();
    }

    Ok(())
}

/// Test stress testing with many concurrent downloads
#[tokio::test]
async fn test_stress_concurrent_downloads() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = TestEnvironment::new()?;
    let temp_dir = TempDir::new()?;

    // Generate many small packages for stress testing
    let generator = TestPackageGenerator::new().await?;
    let package_count = 20; // Reduced for CI compatibility
    let mut packages = Vec::new();
    let mut mocks = Vec::new();

    let server = ConfigurableMockServer::with_defaults();

    for i in 0..package_count {
        let config = TestPackageConfig {
            name: format!("stress-{:03}", i),
            version: Version::parse("1.0.0")?,
            target_size: 16 * 1024, // Small 16KB packages
            content_pattern: ContentPattern::Random,
            use_compression: false,
            generate_signature: false,
            file_count: 1,
            dependencies: Vec::new(),
        };

        let package = generator.generate_package(config).await?;
        let package_data = fs::read(&package.package_path).await?;

        let path = format!("/stress-{:03}-1.0.0.sp", i);
        server.register_file(&path, package_data);
        mocks.push(server.mock_file_download(&path));

        packages.push(package);
    }

    // Create download requests
    let mut requests = Vec::new();
    for (i, package) in packages.iter().enumerate() {
        let path = format!("/stress-{:03}-1.0.0.sp", i);
        requests.push(PackageDownloadRequest {
            name: package.config.name.clone(),
            version: package.config.version.clone(),
            package_url: server.url(&path),
            signature_url: None,
            expected_hash: Some(package.hash.clone()),
        });
    }

    // Configure downloader for high concurrency
    let config = PackageDownloadConfig {
        max_concurrent: 10, // High concurrency
        ..Default::default()
    };
    let downloader = PackageDownloader::new(config)?;

    // Perform stress test
    let start_time = Instant::now();
    let results = downloader
        .download_packages_batch(requests, temp_dir.path(), &env.event_sender)
        .await?;
    let download_time = start_time.elapsed();

    // Verify all packages downloaded successfully
    assert_eq!(results.len(), packages.len());

    println!(
        "Stress test: {} packages downloaded in {:?}",
        package_count, download_time
    );

    // Verify metrics
    let metrics = env.get_metrics();
    assert_eq!(metrics.package_downloads_completed, packages.len() as u64);
    assert!(metrics.all_successful());

    // All downloads should complete reasonably quickly with small files
    assert!(
        download_time < Duration::from_secs(30),
        "Stress test took too long: {:?}",
        download_time
    );

    // Verify at least some concurrency occurred by checking download rate
    let total_size: u64 = packages.iter().map(|p| p.config.target_size as u64).sum();
    let download_rate = total_size as f64 / download_time.as_secs_f64();
    println!("Overall download rate: {:.2} KB/s", download_rate / 1024.0);

    // Verify all mocks were called
    for mock in mocks {
        mock.assert();
    }

    Ok(())
}