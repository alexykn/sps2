//! System initialization and configuration integration tests

use super::common::TestEnvironment;
use sps2_config::Config;
use sps2_events::Event;
use sps2_types::Version;

#[tokio::test]
#[ignore] // Requires /opt/pm SQLite database - fails in CI
async fn test_system_initialization() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnvironment::new().await?;

    // Test that all components are properly initialized
    assert!(env.temp_dir.path().exists());
    assert!(env.config.paths.store_path.as_ref().unwrap().exists());
    assert!(env.temp_dir.path().join("live").exists());

    // Test state manager initialization
    // In a fresh system, an initial state should be automatically created
    let active_state_result = env.ops_ctx.state.get_active_state().await;
    assert!(active_state_result.is_ok()); // Should succeed with initial state

    // List of states should contain exactly one initial state
    let states = env.ops_ctx.state.list_states().await?;
    assert_eq!(states.len(), 1); // Should have one initial state

    Ok(())
}

#[tokio::test]
async fn test_event_system() -> Result<(), Box<dyn std::error::Error>> {
    use sps2_events::Event;
    use sps2_types::Version;

    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();

    // Send some events
    sender
        .send(Event::PackageInstalling {
            name: "hello-world".to_string(),
            version: Version::parse("1.0.0")?,
        })
        .unwrap();

    sender
        .send(Event::DownloadProgress {
            url: "https://example.com/package.sp".to_string(),
            bytes_downloaded: 1024,
            total_bytes: 2048,
        })
        .unwrap();

    // Receive and verify events
    let event1 = receiver.recv().await.unwrap();
    match event1 {
        Event::PackageInstalling { name, version } => {
            assert_eq!(name, "hello-world");
            assert_eq!(version.to_string(), "1.0.0");
        }
        _ => panic!("Unexpected event type"),
    }

    let event2 = receiver.recv().await.unwrap();
    match event2 {
        Event::DownloadProgress {
            url,
            bytes_downloaded,
            total_bytes,
        } => {
            assert_eq!(url, "https://example.com/package.sp");
            assert_eq!(bytes_downloaded, 1024);
            assert_eq!(total_bytes, 2048);
        }
        _ => panic!("Unexpected event type"),
    }

    Ok(())
}

#[tokio::test]
async fn test_configuration_loading() -> Result<(), Box<dyn std::error::Error>> {
    // Test default configuration
    let config = Config::default();
    assert!(config.general.parallel_downloads > 0);
    assert!(config.network.timeout > 0);

    // Test configuration validation
    assert!(!config.security.verify_signatures); // Should be configurable

    Ok(())
}

#[tokio::test]
async fn test_concurrent_operations() -> Result<(), Box<dyn std::error::Error>> {
    use tokio::time::{timeout, Duration};

    let env = TestEnvironment::new().await?;

    // Test that multiple operations can run concurrently
    let futures = vec![
        Box::pin(async {
            // Simulate manifest parsing operation
            env.load_test_manifest("hello-world-1.0.0").await
        }),
        Box::pin(async {
            // Simulate index loading operation
            env.load_test_index().await
        }),
        Box::pin(async {
            // Simulate state listing operation
            env.ops_ctx.state.list_states().await.map(|_| String::new())
        }),
    ];

    // All operations should complete within reasonable time
    let results = timeout(Duration::from_secs(5), futures::future::join_all(futures)).await?;

    // All operations should succeed
    for result in results {
        assert!(result.is_ok(), "Concurrent operation failed");
    }

    Ok(())
}

#[tokio::test]
async fn test_cleanup_and_finalization() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnvironment::new().await?;

    // Test that cleanup works properly
    let temp_path = env.temp_dir.path().to_path_buf();
    assert!(temp_path.exists());

    // Drop the environment (should trigger cleanup)
    drop(env);

    // Note: TempDir cleanup happens when dropped, but we can't easily test
    // that here since it happens asynchronously

    Ok(())
}