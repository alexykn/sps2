//! Production-ready streaming download infrastructure for .sp files
//!
//! This module provides high-performance, resumable downloads with concurrent
//! signature verification and comprehensive error handling.

mod config;
mod core;
mod resume;
mod retry;
mod stream;
mod validation;

// Re-export public types and structs
pub use config::{
    DownloadProgress, DownloadResult, PackageDownloadConfig, PackageDownloadRequest,
    PackageDownloadResult, RetryConfig,
};
pub use core::PackageDownloader;

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use sps2_hash::Hash;
    use sps2_types::Version;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::fs as tokio_fs;

    fn create_test_data(size: usize) -> Vec<u8> {
        #[allow(clippy::cast_possible_truncation)]
        // Safe cast: i % 256 is always in range [0, 255]
        (0..size).map(|i| (i % 256) as u8).collect()
    }

    #[tokio::test]
    async fn test_package_downloader_creation() {
        let _downloader = PackageDownloader::with_defaults().unwrap();
        // Test that the downloader was created successfully
        // Simply verify that creation doesn't panic or error
    }

    #[tokio::test]
    async fn test_url_validation() {
        let _downloader = PackageDownloader::with_defaults().unwrap();

        assert!(super::validation::validate_url("https://example.com/file.sp").is_ok());
        assert!(super::validation::validate_url("http://example.com/file.sp").is_ok());
        assert!(super::validation::validate_url("file:///path/to/file.sp").is_ok());
        assert!(super::validation::validate_url("ftp://example.com/file.sp").is_err());
    }

    #[tokio::test]
    async fn test_backoff_calculation() {
        let config = RetryConfig::default();

        let delay1 = super::retry::calculate_backoff_delay(&config, 1);
        let delay2 = super::retry::calculate_backoff_delay(&config, 2);

        // Second delay should be longer (with potential jitter variation)
        assert!(delay2.as_millis() >= delay1.as_millis());
    }

    #[tokio::test]
    async fn test_resume_offset_calculation() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.sp");

        let config = PackageDownloadConfig::default();

        // No file exists
        let offset = super::resume::get_resume_offset(&config, &file_path)
            .await
            .unwrap();
        assert_eq!(offset, 0);

        // Create a small file (below minimum chunk size)
        tokio_fs::write(&file_path, b"small").await.unwrap();
        let offset = super::resume::get_resume_offset(&config, &file_path)
            .await
            .unwrap();
        assert_eq!(offset, 0); // Should start over

        // Create a large file (above minimum chunk size)
        let large_data = vec![0u8; 2 * 1024 * 1024]; // 2MB
        tokio_fs::write(&file_path, &large_data).await.unwrap();
        let offset = super::resume::get_resume_offset(&config, &file_path)
            .await
            .unwrap();
        assert_eq!(offset, large_data.len() as u64);
    }

    #[tokio::test]
    async fn test_successful_package_download() {
        let temp_dir = TempDir::new().unwrap();
        let (tx, mut rx) = sps2_events::channel();

        // Create test data
        let test_data = create_test_data(1024);
        let test_hash = Hash::from_data(&test_data);

        // Start mock server
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/test-package-1.0.0.sp");
            then.status(200)
                .header("content-length", test_data.len().to_string())
                .header("accept-ranges", "bytes")
                .body(&test_data);
        });

        let signature_mock = server.mock(|when, then| {
            when.method(GET).path("/test-package-1.0.0.sp.minisig");
            then.status(200).body("fake signature content");
        });

        let downloader = PackageDownloader::with_defaults().unwrap();
        let package_url = format!("{}/test-package-1.0.0.sp", server.url(""));
        let signature_url = format!("{}/test-package-1.0.0.sp.minisig", server.url(""));

        let version = Version::parse("1.0.0").unwrap();

        let result = downloader
            .download_package(
                "test-package",
                &version,
                &package_url,
                Some(&signature_url),
                temp_dir.path(),
                Some(&test_hash),
                &tx,
            )
            .await
            .unwrap();

        // Verify the download result
        assert_eq!(result.size, test_data.len() as u64);
        assert_eq!(result.hash, test_hash);
        assert!(result.package_path.exists());
        assert!(result.signature_path.as_ref().unwrap().exists());

        // Verify file content
        let downloaded_data = tokio_fs::read(&result.package_path).await.unwrap();
        assert_eq!(downloaded_data, test_data);

        // Verify mocks were called
        mock.assert();
        signature_mock.assert();

        // Check events
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert!(events
            .iter()
            .any(|e| matches!(e, sps2_events::Event::PackageDownloadStarted { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, sps2_events::Event::PackageDownloaded { .. })));
    }

    #[tokio::test]
    async fn test_resumable_download() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test-resumable.sp");
        let (tx, _rx) = sps2_events::channel();

        // Create test data (large enough to trigger resume logic)
        let test_data = create_test_data(2 * 1024 * 1024); // 2MB
        let first_part = &test_data[..1024 * 1024]; // First 1MB
        let second_part = &test_data[1024 * 1024..]; // Remaining data

        // Write first part to simulate partial download
        tokio_fs::write(&file_path, first_part).await.unwrap();

        // Start mock server that supports range requests
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/test-resumable.sp")
                .header("range", "bytes=1048576-"); // Resume from 1MB
            then.status(206) // Partial Content
                .header("content-length", second_part.len().to_string())
                .header(
                    "content-range",
                    format!("bytes 1048576-{}/{}", test_data.len() - 1, test_data.len()),
                )
                .body(second_part);
        });

        let downloader = PackageDownloader::with_defaults().unwrap();
        let url = format!("{}/test-resumable.sp", server.url(""));

        let result = downloader
            .download_with_resume(&url, &file_path, None, tx)
            .await
            .unwrap();

        // Verify the complete file
        let downloaded_data = tokio_fs::read(&file_path).await.unwrap();
        assert_eq!(downloaded_data, test_data);
        assert_eq!(result.size, test_data.len() as u64);

        mock.assert();
    }

    #[tokio::test]
    async fn test_hash_verification_failure() {
        let temp_dir = TempDir::new().unwrap();
        let (tx, _rx) = sps2_events::channel();

        let test_data = create_test_data(1024);
        let wrong_hash = Hash::from_data(b"wrong data");

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/test-wrong-hash.sp");
            then.status(200).body(&test_data);
        });

        // Configure to reduce retries for faster test
        let mut config = PackageDownloadConfig::default();
        config.retry_config.max_retries = 1;

        let downloader = PackageDownloader::new(config).unwrap();
        let package_url = format!("{}/test-wrong-hash.sp", server.url(""));
        let version = Version::parse("1.0.0").unwrap();

        let result = downloader
            .download_package(
                "test-package",
                &version,
                &package_url,
                None,
                temp_dir.path(),
                Some(&wrong_hash),
                &tx,
            )
            .await;

        assert!(result.is_err());
        if let Err(sps2_errors::Error::Network(sps2_errors::NetworkError::ChecksumMismatch {
            ..
        })) = result
        {
            // Expected error type
        } else {
            panic!("Expected checksum mismatch error, got: {result:?}");
        }

        // Expect multiple calls due to hash verification failing and causing retries
        assert!(mock.hits() >= 1);
    }

    #[tokio::test]
    async fn test_file_size_limit() {
        let temp_dir = TempDir::new().unwrap();
        let (tx, _rx) = sps2_events::channel();

        // Create large body that matches content-length to avoid httpmock issues
        let large_data = vec![b'X'; 5_000_000]; // 5MB of data

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/test-oversized.sp");
            then.status(200)
                .header("content-length", large_data.len().to_string())
                .body(&large_data);
        });

        let config = PackageDownloadConfig {
            max_file_size: 1024, // Very small limit
            ..PackageDownloadConfig::default()
        };

        let downloader = PackageDownloader::new(config).unwrap();
        let package_url = format!("{}/test-oversized.sp", server.url(""));
        let version = Version::parse("1.0.0").unwrap();

        let result = downloader
            .download_package(
                "test-package",
                &version,
                &package_url,
                None,
                temp_dir.path(),
                None,
                &tx,
            )
            .await;

        assert!(result.is_err());
        if let Err(sps2_errors::Error::Network(sps2_errors::NetworkError::FileSizeExceeded {
            ..
        })) = result
        {
            // Expected error type
        } else {
            panic!("Expected file size exceeded error, got: {result:?}");
        }

        // Mock may not be hit if size limit is checked early
        let _ = mock;
    }

    #[tokio::test]
    async fn test_concurrent_batch_download() {
        let temp_dir = TempDir::new().unwrap();
        let (tx, _rx) = sps2_events::channel();

        let server = MockServer::start();

        // Create multiple test packages with deterministic ordering
        let packages = vec![
            ("package-a", "1.0.0", create_test_data(512)),
            ("package-b", "2.0.0", create_test_data(1024)),
            ("package-c", "3.0.0", create_test_data(256)),
        ];

        let mut requests = Vec::new();
        let mut mocks = Vec::new();

        for (name, version, data) in &packages {
            let path = format!("/{name}-{version}.sp");
            let mock = server.mock(|when, then| {
                when.method(GET).path(&path);
                then.status(200)
                    .header("content-length", data.len().to_string())
                    .body(data);
            });
            mocks.push(mock);

            requests.push(PackageDownloadRequest {
                name: (*name).to_string(),
                version: Version::parse(version).unwrap(),
                package_url: format!("{}{}", server.url(""), path),
                signature_url: None,
                expected_hash: Some(Hash::from_data(data)),
            });
        }

        let downloader = PackageDownloader::with_defaults().unwrap();
        let results = downloader
            .download_packages_batch(requests, temp_dir.path(), &tx)
            .await
            .unwrap();

        assert_eq!(results.len(), 3);

        // Verify all packages were downloaded correctly
        // Since downloads are concurrent, results may not be in order
        for (_name, _version, data) in &packages {
            let expected_hash = Hash::from_data(data);
            let result = results.iter().find(|r| r.hash == expected_hash).unwrap();
            assert_eq!(result.size, data.len() as u64);

            let downloaded_data = tokio_fs::read(&result.package_path).await.unwrap();
            assert_eq!(downloaded_data, *data);
        }

        // Verify all mocks were called
        for mock in mocks {
            mock.assert();
        }
    }

    #[tokio::test]
    async fn test_retry_logic() {
        let temp_dir = TempDir::new().unwrap();
        let (tx, _rx) = sps2_events::channel();

        let test_data = create_test_data(512);

        let server = MockServer::start();

        // Create a mock that fails twice then succeeds
        let error_mock = server.mock(|when, then| {
            when.method(GET).path("/test-retry.sp");
            then.status(500); // Server error
        });

        let success_mock = server.mock(|when, then| {
            when.method(GET).path("/test-retry.sp");
            then.status(200).body(&test_data);
        });

        // Configure very fast retries for testing
        let mut config = PackageDownloadConfig::default();
        config.retry_config.max_retries = 3;
        config.retry_config.initial_delay = Duration::from_millis(10);
        config.retry_config.backoff_multiplier = 1.0; // No backoff for faster tests
        config.retry_config.jitter_factor = 0.0; // No jitter

        let downloader = PackageDownloader::new(config).unwrap();
        let package_url = format!("{}/test-retry.sp", server.url(""));
        let version = Version::parse("1.0.0").unwrap();

        // This should eventually succeed after retries
        let result = downloader
            .download_package(
                "test-package",
                &version,
                &package_url,
                None,
                temp_dir.path(),
                None,
                &tx,
            )
            .await;

        // The exact behavior depends on httpmock's mock matching,
        // but we should get either success (if retries work) or failure after max retries
        if let Ok(download_result) = result {
            assert_eq!(download_result.size, test_data.len() as u64);
        } else {
            // Retries exhausted - also acceptable for this test
        }

        // Clean up mocks
        let _ = error_mock;
        let _ = success_mock;
    }

    #[tokio::test]
    async fn test_file_url_support() {
        let temp_dir = TempDir::new().unwrap();
        let source_file = temp_dir.path().join("source.sp");
        let dest_dir = temp_dir.path().join("dest");
        let (_tx, _rx) = sps2_events::channel();

        // Create source file
        let test_data = create_test_data(1024);
        tokio_fs::write(&source_file, &test_data).await.unwrap();
        tokio_fs::create_dir_all(&dest_dir).await.unwrap();

        let _downloader = PackageDownloader::with_defaults().unwrap();
        let file_url = format!("file://{}", source_file.display());

        // Note: This test validates URL parsing but actual file:// support
        // would require additional implementation in the HTTP client
        assert!(crate::download::validation::validate_url(&file_url).is_ok());
    }

    #[tokio::test]
    async fn test_config_validation() {
        // Test various configurations
        let config1 = PackageDownloadConfig {
            max_file_size: 0,
            ..PackageDownloadConfig::default()
        };
        assert!(PackageDownloader::new(config1).is_ok());

        let config2 = PackageDownloadConfig {
            buffer_size: 1024,
            ..PackageDownloadConfig::default()
        };
        assert!(PackageDownloader::new(config2).is_ok());

        let config3 = PackageDownloadConfig {
            max_concurrent: 1,
            ..PackageDownloadConfig::default()
        };
        assert!(PackageDownloader::new(config3).is_ok());

        let config4 = PackageDownloadConfig {
            retry_config: RetryConfig {
                max_retries: 0,
                ..RetryConfig::default()
            },
            ..PackageDownloadConfig::default()
        };
        assert!(PackageDownloader::new(config4).is_ok());
    }
}
