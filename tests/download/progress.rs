//! Download progress tracking tests

use crate::common::*;
use sps2_events::Event;
use sps2_hash::Hash;
use sps2_net::{PackageDownloader, PackageDownloadConfig};
use sps2_types::Version;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::fs;

/// Test single file download with progress tracking
#[tokio::test]
async fn test_single_file_download_with_progress() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = TestEnvironment::new()?;
    let temp_dir = TempDir::new()?;

    // Generate test package
    let generator = TestPackageGenerator::new().await?;
    let config = TestPackageConfig {
        name: "progress-test".to_string(),
        version: Version::parse("1.0.0")?,
        target_size: 256 * 1024, // 256KB
        content_pattern: ContentPattern::Random,
        use_compression: true,
        generate_signature: true,
        file_count: 5,
        dependencies: Vec::new(),
    };
    let package = generator.generate_package(config).await?;

    // Read package data for mock server
    let package_data = fs::read(&package.package_path).await?;
    let signature_data = fs::read(package.signature_path.as_ref().unwrap()).await?;

    // Set up mock server
    let server = ConfigurableMockServer::with_defaults();
    server.register_file("/progress-test-1.0.0.sp", package_data.clone());
    server.register_file("/progress-test-1.0.0.sp.minisig", signature_data.clone());

    let package_mock = server.mock_file_download("/progress-test-1.0.0.sp");
    let signature_mock = server.mock_signature_download(
        "/progress-test-1.0.0.sp.minisig",
        &String::from_utf8_lossy(&signature_data),
    );

    // Download with progress tracking
    let downloader = PackageDownloader::with_defaults()?;
    let package_url = server.url("/progress-test-1.0.0.sp");
    let signature_url = server.url("/progress-test-1.0.0.sp.minisig");

    let result = downloader
        .download_package(
            "progress-test",
            &Version::parse("1.0.0")?,
            &package_url,
            Some(&signature_url),
            temp_dir.path(),
            Some(&package.hash),
            &env.event_sender,
        )
        .await?;

    // Verify download completed successfully
    assert_eq!(result.size, package_data.len() as u64);
    assert_eq!(result.hash, package.hash);
    assert!(result.package_path.exists());
    assert!(result.signature_path.as_ref().unwrap().exists());

    // Collect and verify events
    let events = env
        .collect_events_with_timeout(Duration::from_millis(500))
        .await;
    EventVerifier::verify_package_download_sequence(&events, 1)?;

    // Verify progress events were sent
    let progress_count = EventVerifier::count_events_of_type(&events, |e| {
        matches!(e, Event::DownloadProgress { .. })
    });
    assert!(progress_count > 0, "Expected progress events, got none");

    // Verify performance metrics
    let metrics = env.get_metrics();
    assert!(metrics.success_rate() > 95.0);
    assert!(metrics.average_download_speed() > 0.0);

    // Verify mocks were called
    package_mock.assert();
    signature_mock.assert();

    Ok(())
}

/// Test download with network simulation (slow connection)
#[tokio::test]
async fn test_download_with_network_simulation() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = TestEnvironment::new()?;
    let temp_dir = TempDir::new()?;

    // Generate test package
    let generator = TestPackageGenerator::new().await?;
    let config = TestPackageConfig {
        name: "slow-test".to_string(),
        version: Version::parse("1.0.0")?,
        target_size: 512 * 1024, // 512KB
        content_pattern: ContentPattern::Text,
        use_compression: true,
        generate_signature: false,
        file_count: 10,
        dependencies: Vec::new(),
    };
    let package = generator.generate_package(config).await?;
    let package_data = fs::read(&package.package_path).await?;

    // Set up throttled server for realistic bandwidth simulation
    let throttled_server = ThrottledHttpServer::new(
        package_data.clone(),
        50_000, // 50KB/s for more noticeable throttling
    )
    .await?;

    // Download with network simulation
    let downloader = PackageDownloader::with_defaults()?;
    let package_url = throttled_server.url();

    let start_time = Instant::now();
    let result = downloader
        .download_package(
            "slow-test",
            &Version::parse("1.0.0")?,
            &package_url,
            None,
            temp_dir.path(),
            Some(&package.hash),
            &env.event_sender,
        )
        .await?;
    let download_time = start_time.elapsed();

    // Verify download succeeded despite slow connection
    assert_eq!(result.hash, package.hash);

    // Should take reasonable time given the throttling
    // With 512KB at 50KB/s, should take around 10+ seconds, but we'll be more conservative
    println!("Download with throttling took {:?}", download_time);
    println!(
        "File size: {} bytes, Rate: {} KB/s",
        package_data.len(),
        throttled_server.bytes_per_second() / 1024
    );

    // At 50KB/s, 512KB should take ~10 seconds, but let's be conservative and expect at least 5 seconds
    let expected_min_time = Duration::from_secs(5);
    assert!(
        download_time > expected_min_time,
        "Download should take at least {:?} at 50KB/s for {}KB file, but took {:?}",
        expected_min_time,
        package_data.len() / 1024,
        download_time
    );

    // Verify events show progress updates
    let events = env
        .collect_events_with_timeout(Duration::from_millis(500))
        .await;
    let progress_count = EventVerifier::count_events_of_type(&events, |e| {
        matches!(e, Event::DownloadProgress { .. })
    });
    assert!(
        progress_count > 1,
        "Expected multiple progress updates for slow download"
    );

    // Throttled server doesn't need assertion - it automatically serves the content
    Ok(())
}

/// Test resumable download with interruption simulation
#[tokio::test]
async fn test_resumable_download() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = TestEnvironment::new()?;
    let temp_dir = TempDir::new()?;

    // Generate larger test package for meaningful resume test
    let generator = TestPackageGenerator::new().await?;
    let config = TestPackageConfig {
        name: "resume-test".to_string(),
        version: Version::parse("1.0.0")?,
        target_size: 1024 * 1024, // 1MB
        content_pattern: ContentPattern::Sequential,
        use_compression: false, // No compression for predictable size
        generate_signature: false,
        file_count: 20,
        dependencies: Vec::new(),
    };
    let package = generator.generate_package(config).await?;
    let package_data = fs::read(&package.package_path).await?;

    // Set up mock server with partial content support
    let server = ConfigurableMockServer::with_defaults();
    server.register_file("/resume-test-1.0.0.sp", package_data.clone());
    let mock = server.mock_resumable_download("/resume-test-1.0.0.sp");

    // Configure downloader for resumable downloads
    let config = PackageDownloadConfig {
        enable_resume: true,
        ..Default::default()
    };
    let downloader = PackageDownloader::new(config)?;

    // First, create a partial file to simulate interrupted download
    let download_path = temp_dir.path().join("resume-test-1.0.0.sp");
    let partial_size = package_data.len() / 3; // One third of the file
    fs::write(&download_path, &package_data[..partial_size]).await?;

    // Now resume the download
    let package_url = server.url("/resume-test-1.0.0.sp");
    let result = downloader
        .download_package(
            "resume-test",
            &Version::parse("1.0.0")?,
            &package_url,
            None,
            temp_dir.path(),
            Some(&package.hash),
            &env.event_sender,
        )
        .await?;

    // Verify resume completed successfully
    assert_eq!(result.size, package_data.len() as u64);
    assert_eq!(result.hash, package.hash);

    // Verify full file content
    let downloaded_data = fs::read(&download_path).await?;
    assert_eq!(downloaded_data, package_data);

    // Verify events include resume information
    let events = env
        .collect_events_with_timeout(Duration::from_millis(500))
        .await;
    
    // Should have progress events
    let progress_count = EventVerifier::count_events_of_type(&events, |e| {
        matches!(e, Event::DownloadProgress { .. })
    });
    assert!(progress_count > 0);

    mock.assert();
    Ok(())
}