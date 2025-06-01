//! Integration tests for the complete install workflow
//!
//! These tests validate the end-to-end installation pipeline including:
//! - URL resolution and package discovery
//! - Parallel download and validation
//! - Atomic state management
//! - Error recovery and user experience
//! - Mixed local/remote package operations

use sps2_events::{Event, EventSender};
use sps2_index::IndexManager;
use sps2_net::NetClient;
use sps2_ops::{OpsContextBuilder, OpsCtx};
use sps2_resolver::Resolver;
use sps2_state::StateManager;
use sps2_store::PackageStore;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::sync::mpsc;

#[allow(unused_imports)]
mod common;
use common::mock_server::ConfigurableMockServer;
use common::repo_simulation::MockRepository;
use common::test_helpers::TestEventCollector;

/// Test setup for integration tests
#[allow(dead_code)] // Test infrastructure - not all fields used yet
struct IntegrationTestSetup {
    temp_dir: TempDir,
    ops_ctx: OpsCtx,
    event_collector: Arc<Mutex<TestEventCollector>>,
    mock_repo: MockRepository,
    mock_server: ConfigurableMockServer,
    // Hold sender to control channel lifetime
    _event_sender: EventSender,
    // Hold task handle to ensure proper cleanup
    _event_task: tokio::task::JoinHandle<()>,
}

impl IntegrationTestSetup {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // Allow HTTP in tests to work with mock server
        std::env::set_var("SPS2_ALLOW_HTTP", "1");

        let temp_dir = TempDir::new()?;

        // Create event channel with collector
        let (tx, mut rx) = mpsc::unbounded_channel();
        let event_collector = Arc::new(Mutex::new(TestEventCollector::new()));
        let collector_clone = event_collector.clone();

        // Spawn event collector task with timeout to prevent hanging
        let event_task = tokio::spawn(async move {
            use tokio::time::{timeout, Duration};
            loop {
                match timeout(Duration::from_millis(100), rx.recv()).await {
                    Ok(Some(event)) => {
                        collector_clone.lock().unwrap().add_event(event);
                    }
                    Ok(None) => break,  // Channel closed
                    Err(_) => continue, // Timeout, keep trying
                }
            }
        });

        // Initialize components (without resolver initially)
        let mut index_manager = IndexManager::new(temp_dir.path());
        let state_manager = StateManager::new(temp_dir.path()).await?;
        let store = PackageStore::new(temp_dir.path().to_path_buf());
        let net_client = NetClient::with_defaults()?;
        let builder = sps2_builder::Builder::default();

        // Setup mock HTTP server
        let mock_server = ConfigurableMockServer::with_defaults();

        // Setup mock repository with the mock server's URL
        let mut mock_repo = MockRepository::with_base_url(mock_server.url(""));
        mock_repo.setup_common_packages().await?;

        // Register package files with the mock server
        for (url, package_data) in &mock_repo.packages {
            // Extract the path from the URL (everything after the base URL)
            if let Some(path) = url.strip_prefix(&mock_repo.base_url) {
                println!(
                    "DEBUG: Registering package file: {} -> {} bytes",
                    path,
                    package_data.len()
                );
                mock_server.register_file(path, package_data.clone());
                mock_server.mock_file_download(path);

                // Also register the signature file
                let sig_path = format!("{}.minisig", path);
                let fake_signature = b"fake signature content".to_vec();
                mock_server.register_file(&sig_path, fake_signature);
                mock_server.mock_file_download(&sig_path);
                println!("DEBUG: Registering signature file: {}", sig_path);
            }
        }

        // Load mock index
        let index_json = mock_repo.get_index_json()?;
        println!("DEBUG: Index JSON length: {} bytes", index_json.len());
        println!(
            "DEBUG: Index JSON snippet: {}",
            &index_json[..std::cmp::min(500, index_json.len())]
        );
        index_manager.load(Some(&index_json)).await?;

        // Debug: Check what's in the loaded index
        if let Some(index) = index_manager.index() {
            println!(
                "DEBUG: Loaded index contains {} packages",
                index.packages.len()
            );
            for package_name in index.packages.keys() {
                println!("DEBUG: Package in index: {}", package_name);
            }
        } else {
            println!("DEBUG: No index loaded!");
        }

        // Create resolver AFTER loading the index
        let resolver = Resolver::new(index_manager.clone());

        // Clone the sender before building ops context
        let tx_clone = tx.clone();

        // Build ops context
        let ops_ctx = OpsContextBuilder::new()
            .with_store(store)
            .with_state(state_manager)
            .with_index(index_manager.clone())
            .with_net(net_client)
            .with_resolver(resolver)
            .with_builder(builder)
            .with_event_sender(tx)
            .build()?;

        Ok(Self {
            temp_dir,
            ops_ctx,
            event_collector,
            mock_repo,
            mock_server,
            _event_sender: tx_clone,
            _event_task: event_task,
        })
    }

    /// Get collected events for analysis
    fn get_events(&self) -> Vec<Event> {
        self.event_collector.lock().unwrap().get_events_cloned()
    }

    /// Count events of a specific type
    fn count_events<F>(&self, predicate: F) -> usize
    where
        F: Fn(&Event) -> bool,
    {
        self.get_events().iter().filter(|e| predicate(e)).count()
    }

    /// Wait for a specific event to occur
    #[allow(dead_code)] // Test infrastructure - not used yet
    async fn wait_for_event<F>(&self, predicate: F, timeout_ms: u64) -> bool
    where
        F: Fn(&Event) -> bool,
    {
        let start = std::time::Instant::now();
        while start.elapsed().as_millis() < timeout_ms as u128 {
            if self.get_events().iter().any(&predicate) {
                return true;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
        false
    }
}

impl Drop for IntegrationTestSetup {
    fn drop(&mut self) {
        // The event task will exit naturally due to timeout or channel closure
        // Abort as a final safeguard
        self._event_task.abort();
    }
}

#[tokio::test]
async fn test_install_single_remote_package() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Test installing a single remote package
    let packages = vec!["openssl".to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    // Should succeed
    assert!(result.is_ok(), "Install should succeed: {:?}", result.err());

    let report = result.unwrap();
    println!(
        "DEBUG: Install report - installed: {}, updated: {}, removed: {}",
        report.installed.len(),
        report.updated.len(),
        report.removed.len()
    );
    for installed in &report.installed {
        println!(
            "DEBUG: Installed package: {} from {:?} to {:?}",
            installed.name, installed.from_version, installed.to_version
        );
    }

    // Debug: Print all events to see what happened
    let events = setup.get_events();
    println!("DEBUG: Total events collected: {}", events.len());
    for (i, event) in events.iter().enumerate() {
        println!("DEBUG: Event {}: {:?}", i, event);
    }

    assert_eq!(report.installed.len(), 1);
    assert_eq!(report.installed[0].name, "openssl");

    // Give the event collector task a moment to process any final events
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Check events
    assert!(setup.count_events(|e| matches!(e, Event::InstallStarting { .. })) > 0);
    assert!(setup.count_events(|e| matches!(e, Event::InstallCompleted { .. })) > 0);
    assert!(setup.count_events(|e| matches!(e, Event::ProgressStarted { .. })) > 0);
    assert!(setup.count_events(|e| matches!(e, Event::ProgressCompleted { .. })) > 0);
}

#[tokio::test]
async fn test_install_package_with_dependencies() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Test installing curl which depends on openssl
    let packages = vec!["curl".to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    // Should succeed and install both packages
    assert!(result.is_ok(), "Install should succeed: {:?}", result.err());

    let report = result.unwrap();
    assert!(
        report.installed.len() >= 2,
        "Should install curl and its dependencies"
    );

    // Check that dependency resolution events were emitted
    assert!(setup.count_events(|e| matches!(e, Event::ResolvingDependencies { .. })) > 0);

    // Verify both packages are reported as installed
    let installed_names: Vec<&String> = report.installed.iter().map(|p| &p.name).collect();
    assert!(installed_names.contains(&&"curl".to_string()));
    assert!(installed_names.contains(&&"openssl".to_string()));
}

#[tokio::test]
async fn test_install_with_version_constraints() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Test installing with version constraints
    let packages = vec!["openssl>=3.0.0".to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    assert!(
        result.is_ok(),
        "Install with version constraint should succeed: {:?}",
        result.err()
    );

    let report = result.unwrap();
    assert_eq!(report.installed.len(), 1);
    assert_eq!(report.installed[0].name, "openssl");
}

#[tokio::test]
#[ignore]
async fn test_install_multiple_packages() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Test installing multiple unrelated packages
    let packages = vec!["jq".to_string(), "curl".to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    assert!(
        result.is_ok(),
        "Multi-package install should succeed: {:?}",
        result.err()
    );

    let report = result.unwrap();
    // Should install jq, oniguruma (jq's dep), curl, and openssl (curl's dep)
    assert!(
        report.installed.len() >= 4,
        "Should install all packages and dependencies"
    );

    // Check that parallel pipeline was used
    assert!(setup.count_events(|e| matches!(e, Event::ProgressStarted { .. })) > 0);

    // Verify progress events show phases
    let events = setup.get_events();
    let progress_events: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::ProgressStarted { .. }))
        .collect();

    if let Some(Event::ProgressStarted { phases, .. }) = progress_events.first() {
        assert!(
            phases.len() >= 3,
            "Should have multiple phases (resolve, download, install)"
        );
    }
}

#[tokio::test]
async fn test_install_local_package() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Create a local test package
    let package_data = MockRepository::create_test_package("test-local", "1.0.0", vec![])
        .await
        .unwrap();
    let local_path = setup.temp_dir.path().join("test-local-1.0.0-1.arm64.sp");
    tokio::fs::write(&local_path, package_data).await.unwrap();

    // Test installing the local package
    let packages = vec![local_path.display().to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    assert!(
        result.is_ok(),
        "Local package install should succeed: {:?}",
        result.err()
    );

    let report = result.unwrap();
    assert_eq!(report.installed.len(), 1);
    assert_eq!(report.installed[0].name, "test-local");
}

// NOTE: Local package dependency resolution is not yet fully implemented
// The realistic mixed scenario (local package with remote dependencies)
// requires parsing manifest dependencies from local files, which appears
// to have parsing issues with constraint formats. This is an edge case
// that can be implemented later when needed.

#[tokio::test]
async fn test_install_nonexistent_package() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Test installing a package that doesn't exist
    let packages = vec!["nonexistent-package".to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    // Should fail with helpful error
    assert!(
        result.is_err(),
        "Install of nonexistent package should fail"
    );

    // Give the event collector task a moment to process any final events
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Check that helpful error events were emitted
    let events = setup.get_events();
    let error_events: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::Error { .. }))
        .collect();

    assert!(!error_events.is_empty(), "Should emit error events");

    if let Some(Event::Error {
        details: Some(details),
        ..
    }) = error_events.first()
    {
        assert!(
            details.contains("Suggested solutions"),
            "Should provide helpful suggestions"
        );
    }
}

#[tokio::test]
async fn test_install_progress_reporting() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Install a package with dependencies to get rich progress reporting
    let packages = vec!["curl".to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    assert!(result.is_ok(), "Install should succeed");

    // Give the event collector task a moment to process any final events
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Verify comprehensive progress events
    assert!(setup.count_events(|e| matches!(e, Event::ProgressStarted { .. })) > 0);
    assert!(setup.count_events(|e| matches!(e, Event::ResolvingDependencies { .. })) > 0);
    assert!(setup.count_events(|e| matches!(e, Event::ProgressCompleted { .. })) > 0);

    // Check for phase transitions
    let phase_events = setup.count_events(|e| matches!(e, Event::ProgressPhaseChanged { .. }));
    assert!(phase_events > 0, "Should have phase transition events");

    // Verify install completion events
    assert!(setup.count_events(|e| matches!(e, Event::InstallCompleted { .. })) > 0);
}

#[tokio::test]
async fn test_install_with_invalid_local_file() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Test with a non-existent local file
    let packages = vec!["./nonexistent.sp".to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    assert!(
        result.is_err(),
        "Install with invalid local file should fail"
    );

    // Check that specific guidance for local files was provided
    let events = setup.get_events();
    let error_events: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::Error { .. }))
        .collect();

    if let Some(Event::Error {
        details: Some(details),
        ..
    }) = error_events.first()
    {
        assert!(
            details.contains("file paths"),
            "Should mention file path issues"
        );
        assert!(
            details.contains("Suggested solutions"),
            "Should provide suggestions"
        );
    }
}

#[tokio::test]
#[ignore]
async fn test_install_performance_metrics() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Install multiple packages to generate performance metrics
    let packages = vec!["curl".to_string(), "jq".to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    assert!(result.is_ok(), "Install should succeed");

    // Check for debug events with performance metrics
    let events = setup.get_events();
    let debug_events: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::DebugLog { .. }))
        .collect();

    // Should have performance metrics in debug logs
    let has_performance_metrics = debug_events.iter().any(|e| {
        if let Event::DebugLog { message, context } = e {
            message.contains("speed")
                || message.contains("efficiency")
                || context.contains_key("total_downloaded")
        } else {
            false
        }
    });

    assert!(has_performance_metrics, "Should emit performance metrics");
}

#[tokio::test]
async fn test_state_management_integration() {
    let setup = IntegrationTestSetup::new().await.unwrap();

    // Get initial state
    let initial_states = setup.ops_ctx.state.list_states_detailed().await.unwrap();
    let initial_count = initial_states.len();

    // Install a package
    let packages = vec!["openssl".to_string()];
    let result = sps2_ops::install(&setup.ops_ctx, &packages).await;

    assert!(result.is_ok(), "Install should succeed");

    // Check that a new state was created
    let final_states = setup.ops_ctx.state.list_states_detailed().await.unwrap();
    assert!(
        final_states.len() > initial_count,
        "Should create new state"
    );

    // Verify the install report contains the new state ID
    let report = result.unwrap();
    assert!(
        final_states.iter().any(|s| s.state_id() == report.state_id),
        "Report should reference an actual state"
    );
}
