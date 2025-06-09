//! System setup and initialization

use crate::error::CliError;
use sps2_builder::Builder;
use sps2_config::Config;
use sps2_index::IndexManager;
use sps2_net::NetClient;
use sps2_resolver::Resolver;
use sps2_state::StateManager;
use sps2_store::PackageStore;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// System setup and component initialization
pub struct SystemSetup {
    config: Config,
    store: Option<PackageStore>,
    state: Option<StateManager>,
    index: Option<IndexManager>,
    net: Option<NetClient>,
    resolver: Option<Resolver>,
    builder: Option<Builder>,
}

impl SystemSetup {
    /// Create new system setup
    pub fn new(config: Config) -> Self {
        Self {
            config,
            store: None,
            state: None,
            index: None,
            net: None,
            resolver: None,
            builder: None,
        }
    }

    /// Initialize all system components
    pub async fn initialize(&mut self) -> Result<(), CliError> {
        info!("Initializing sps2 system components");

        // Check and create system directories
        self.ensure_system_directories().await?;

        // Initialize core components
        self.init_store().await?;
        self.init_state().await?;
        self.init_index().await?;
        self.init_net().await?;
        self.init_resolver().await?;
        self.init_builder().await?;

        // Perform startup maintenance
        self.startup_maintenance().await?;

        info!("System initialization completed");
        Ok(())
    }

    /// Get package store
    pub fn store(&self) -> &PackageStore {
        self.store.as_ref().expect("store not initialized")
    }

    /// Get state manager
    pub fn state(&self) -> &StateManager {
        self.state.as_ref().expect("state not initialized")
    }

    /// Get index manager
    pub fn index(&self) -> &IndexManager {
        self.index.as_ref().expect("index not initialized")
    }

    /// Get network client
    pub fn net(&self) -> &NetClient {
        self.net.as_ref().expect("net not initialized")
    }

    /// Get resolver
    pub fn resolver(&self) -> &Resolver {
        self.resolver.as_ref().expect("resolver not initialized")
    }

    /// Get builder
    pub fn builder(&self) -> Builder {
        self.builder
            .as_ref()
            .expect("builder not initialized")
            .clone()
    }

    /// Ensure required system directories exist
    async fn ensure_system_directories(&self) -> Result<(), CliError> {
        let required_dirs = [
            "/opt/pm",
            "/opt/pm/store",
            "/opt/pm/states",
            "/opt/pm/live",
            "/opt/pm/logs",
            "/opt/pm/keys",
        ];

        for dir in &required_dirs {
            let path = Path::new(dir);
            if !path.exists() {
                debug!("Creating directory: {}", dir);
                tokio::fs::create_dir_all(path)
                    .await
                    .map_err(|e| CliError::Setup(format!("Failed to create {}: {}", dir, e)))?;
            }
        }

        // Check permissions on critical paths
        self.check_permissions().await?;

        Ok(())
    }

    /// Check permissions on system directories
    async fn check_permissions(&self) -> Result<(), CliError> {
        let paths_to_check = ["/opt/pm", "/opt/pm/store", "/opt/pm/states", "/opt/pm/live"];

        for path in &paths_to_check {
            let metadata = tokio::fs::metadata(path)
                .await
                .map_err(|e| CliError::Setup(format!("Cannot access {}: {}", path, e)))?;

            // Check if we can write to the directory
            if metadata.permissions().readonly() {
                return Err(CliError::Setup(format!("No write permission for {}", path)));
            }
        }

        Ok(())
    }

    /// Initialize package store
    async fn init_store(&mut self) -> Result<(), CliError> {
        debug!("Initializing package store");
        let store_path = Path::new("/opt/pm/store");
        let store = PackageStore::new(store_path.to_path_buf());

        self.store = Some(store);
        Ok(())
    }

    /// Initialize state manager
    async fn init_state(&mut self) -> Result<(), CliError> {
        debug!("Initializing state manager");
        let state_path = Path::new("/opt/pm");
        let state = StateManager::new(state_path)
            .await
            .map_err(|e| CliError::Setup(format!("Failed to initialize state: {}", e)))?;

        self.state = Some(state);
        Ok(())
    }

    /// Initialize index manager
    async fn init_index(&mut self) -> Result<(), CliError> {
        debug!("Initializing index manager");
        let cache_path = Path::new("/opt/pm");
        let mut index = IndexManager::new(cache_path);

        // Try to load cached index
        match index.load(None).await {
            Ok(()) => {
                debug!("Loaded cached index");
            }
            Err(e) => {
                warn!("Failed to load cached index, will need reposync: {}", e);
                // Create empty index for now
                let empty_index = sps2_index::Index::new();
                let json = empty_index
                    .to_json()
                    .map_err(|e| CliError::Setup(format!("Failed to create empty index: {}", e)))?;
                index
                    .load(Some(&json))
                    .await
                    .map_err(|e| CliError::Setup(format!("Failed to load empty index: {}", e)))?;
            }
        }

        self.index = Some(index);
        Ok(())
    }

    /// Initialize network client
    async fn init_net(&mut self) -> Result<(), CliError> {
        debug!("Initializing network client");

        // Create NetConfig from our configuration
        let net_config = sps2_net::NetConfig {
            timeout: std::time::Duration::from_secs(self.config.network.timeout),
            connect_timeout: std::time::Duration::from_secs(30),
            pool_idle_timeout: std::time::Duration::from_secs(90),
            pool_max_idle_per_host: 10,
            retry_count: self.config.network.retries,
            retry_delay: std::time::Duration::from_secs(self.config.network.retry_delay),
            user_agent: format!("sps2/{}", env!("CARGO_PKG_VERSION")),
        };

        let net = sps2_net::NetClient::new(net_config)
            .map_err(|e| CliError::Setup(format!("Failed to create network client: {}", e)))?;

        self.net = Some(net);
        Ok(())
    }

    /// Initialize resolver
    async fn init_resolver(&mut self) -> Result<(), CliError> {
        debug!("Initializing resolver");
        let index = self.index.as_ref().unwrap().clone();
        let resolver = Resolver::new(index);

        self.resolver = Some(resolver);
        Ok(())
    }

    /// Initialize builder
    async fn init_builder(&mut self) -> Result<(), CliError> {
        debug!("Initializing builder");
        let net = self.net.as_ref().unwrap().clone();
        let builder = Builder::new().with_net(net);

        self.builder = Some(builder);
        Ok(())
    }

    /// Perform startup maintenance tasks
    async fn startup_maintenance(&mut self) -> Result<(), CliError> {
        debug!("Performing startup maintenance");

        // Check if garbage collection is needed (>7 days since last GC)
        let state = self.state.as_ref().unwrap();
        if self.should_run_startup_gc(state).await? {
            info!("Running startup garbage collection");

            // Clean up old states
            let cleaned_states = state
                .cleanup_old_states(10)
                .await
                .map_err(|e| CliError::Setup(format!("Startup GC failed: {}", e)))?;

            // Clean up orphaned packages
            let store = self.store.as_ref().unwrap();
            let cleaned_packages = store
                .garbage_collect()
                .map_err(|e| CliError::Setup(format!("Startup GC failed: {}", e)))?;

            if !cleaned_states.is_empty() || cleaned_packages > 0 {
                info!(
                    "Startup GC: cleaned {} states and {} packages",
                    cleaned_states.len(),
                    cleaned_packages
                );
            }

            // Update GC timestamp after successful cleanup
            self.write_last_gc_timestamp().await?;
        }

        // Clean orphaned staging directories
        self.clean_orphaned_staging().await?;

        Ok(())
    }

    /// Check if startup GC should run
    async fn should_run_startup_gc(&self, _state: &StateManager) -> Result<bool, CliError> {
        // Check if last GC was >7 days ago by reading timestamp file
        match self.read_last_gc_timestamp().await {
            Ok(last_gc) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let seven_days_ago = now.saturating_sub(7 * 24 * 60 * 60);
                Ok(last_gc < seven_days_ago)
            }
            Err(_) => {
                // If we can't read the timestamp file, assume GC is needed
                debug!("No GC timestamp found, running startup GC");
                Ok(true)
            }
        }
    }

    /// Clean up orphaned staging directories
    async fn clean_orphaned_staging(&self) -> Result<(), CliError> {
        let states_dir = Path::new("/opt/pm/states");
        if !states_dir.exists() {
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(states_dir)
            .await
            .map_err(|e| CliError::Setup(format!("Failed to read states directory: {}", e)))?;

        let mut cleaned = 0;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| CliError::Setup(format!("Failed to read directory entry: {}", e)))?
        {
            let file_name = entry.file_name();
            if let Some(name) = file_name.to_str() {
                if name.starts_with("staging-") {
                    debug!("Removing orphaned staging directory: {}", name);
                    if let Err(e) = tokio::fs::remove_dir_all(entry.path()).await {
                        warn!(
                            "Failed to remove orphaned staging directory {}: {}",
                            name, e
                        );
                    } else {
                        cleaned += 1;
                    }
                }
            }
        }

        if cleaned > 0 {
            info!("Cleaned {} orphaned staging directories", cleaned);
        }

        Ok(())
    }

    /// Read the last GC timestamp from file
    async fn read_last_gc_timestamp(&self) -> Result<u64, CliError> {
        let timestamp_path = self.gc_timestamp_path();
        let content = tokio::fs::read_to_string(&timestamp_path)
            .await
            .map_err(|e| CliError::Setup(format!("Failed to read GC timestamp: {}", e)))?;

        content
            .trim()
            .parse::<u64>()
            .map_err(|e| CliError::Setup(format!("Invalid GC timestamp format: {}", e)))
    }

    /// Write the current timestamp as the last GC time
    async fn write_last_gc_timestamp(&self) -> Result<(), CliError> {
        let timestamp_path = self.gc_timestamp_path();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        tokio::fs::write(&timestamp_path, now.to_string())
            .await
            .map_err(|e| CliError::Setup(format!("Failed to write GC timestamp: {}", e)))?;

        debug!("Updated GC timestamp: {}", now);
        Ok(())
    }

    /// Get the path to the GC timestamp file
    fn gc_timestamp_path(&self) -> PathBuf {
        Path::new("/opt/pm/.last_gc_timestamp").to_path_buf()
    }

    /// Update GC timestamp after successful cleanup
    #[allow(dead_code)] // Instance method for future use
    pub async fn update_gc_timestamp(&self) -> Result<(), CliError> {
        self.write_last_gc_timestamp().await
    }

    /// Update GC timestamp - public static method for ops crate
    pub async fn update_gc_timestamp_static() -> Result<(), CliError> {
        let timestamp_path = std::path::Path::new("/opt/pm/.last_gc_timestamp");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        tokio::fs::write(timestamp_path, now.to_string())
            .await
            .map_err(|e| CliError::Setup(format!("Failed to write GC timestamp: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_system_setup_creation() {
        let config = Config::default();
        let setup = SystemSetup::new(config);

        // Initially no components should be initialized
        assert!(setup.store.is_none());
        assert!(setup.state.is_none());
        assert!(setup.index.is_none());
    }

    #[tokio::test]
    async fn test_directory_creation() {
        let _temp = tempdir().unwrap();
        let config = Config::default();

        // This test would need to mock the /opt/pm path for proper testing
        // For now, just verify the setup structure
        let setup = SystemSetup::new(config);
        assert!(setup.store.is_none());
    }
}
