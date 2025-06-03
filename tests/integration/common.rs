//! Common test utilities for integration tests

use std::path::PathBuf;
use tempfile::TempDir;
use tokio::fs;
use sps2_config::Config;
use sps2_events::{EventReceiver, EventSender};
use sps2_ops::OpsCtx;
use sps2_state::StateManager;
use sps2_store::PackageStore;

pub struct TestEnvironment {
    pub temp_dir: TempDir,
    pub config: Config,
    pub ops_ctx: OpsCtx,
    #[allow(dead_code)] // Used in event-based integration tests
    pub event_sender: EventSender,
    #[allow(dead_code)] // Used in event-based integration tests
    pub event_receiver: EventReceiver,
}

impl TestEnvironment {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let base_path = temp_dir.path();

        // Set up test directory structure
        let store_path = base_path.join("store");
        let state_path = base_path.join("state.sqlite");
        let live_path = base_path.join("live");
        let cache_path = base_path.join("cache");

        fs::create_dir_all(&store_path).await?;
        fs::create_dir_all(&live_path).await?;
        fs::create_dir_all(&cache_path).await?;

        // Create test configuration
        let mut config = Config::default();
        config.paths.store_path = Some(store_path.clone());
        config.paths.state_path = Some(state_path);
        config.general.parallel_downloads = 2; // Smaller for tests
        config.network.timeout = 30;
        config.security.verify_signatures = false; // Disable for tests

        // Create event channel
        let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

        // Initialize components
        let state = StateManager::new(base_path).await?;
        let store = PackageStore::new(store_path);
        let index = sps2_index::IndexManager::new(base_path);
        let net = sps2_net::NetClient::with_defaults()?;
        let resolver = sps2_resolver::Resolver::new(index.clone());
        let builder = sps2_builder::Builder::new();

        let ops_ctx = OpsCtx::new(
            store,
            state,
            index,
            net,
            resolver,
            builder,
            event_sender.clone(),
        );

        Ok(Self {
            temp_dir,
            config,
            ops_ctx,
            event_sender,
            event_receiver,
        })
    }

    pub fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests")
            .join("fixtures")
    }

    pub async fn load_test_manifest(
        &self,
        name: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let manifest_path = Self::fixtures_path()
            .join("manifests")
            .join(format!("{name}.toml"));
        Ok(fs::read_to_string(manifest_path).await?)
    }

    #[allow(dead_code)] // Used in Starlark recipe testing
    pub async fn load_test_recipe(
        &self,
        name: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let recipe_path = Self::fixtures_path()
            .join("recipes")
            .join(format!("{name}.star"));
        Ok(fs::read_to_string(recipe_path).await?)
    }

    pub async fn load_test_index(&self) -> Result<String, Box<dyn std::error::Error>> {
        let index_path = Self::fixtures_path().join("index").join("packages.json");
        Ok(fs::read_to_string(index_path).await?)
    }
}