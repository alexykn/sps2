//! System setup and initialization

use crate::error::CliError;
use sps2_builder::Builder;
use sps2_config::{fixed_paths, Config};
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

        // Initialize platform cache for optimized tool discovery
        self.init_platform_cache().await?;

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
            fixed_paths::PREFIX,
            fixed_paths::STORE_DIR,
            fixed_paths::STATES_DIR,
            fixed_paths::LIVE_DIR,
            fixed_paths::LOGS_DIR,
            fixed_paths::KEYS_DIR,
        ];

        for dir in &required_dirs {
            let path = Path::new(dir);
            if !path.exists() {
                debug!("Creating directory: {}", dir);
                tokio::fs::create_dir_all(path)
                    .await
                    .map_err(|e| CliError::Setup(format!("Failed to create {dir}: {e}")))?;
            }
        }

        // Seed default repositories and keys on first run
        self.seed_default_repositories_and_keys().await?;

        // Check permissions on critical paths
        self.check_permissions().await?;

        Ok(())
    }

    /// Seed default repositories and embedded public keys on first run
    async fn seed_default_repositories_and_keys(&self) -> Result<(), CliError> {
        use sps2_config::fixed_paths;
        use tokio::fs;

        // Initialize repos in config if empty
        // For now, write defaults into config file only if none are present
        // Embedded public key placeholder (replace with real key for production)
        let bootstrap_key = "RWSGOq2NVecA2UPNdBUZykp1MLhfMmkAK/SZSjK3bpq2q7I8LbSVVBDm".to_string();

        // Ensure keys dir exists
        fs::create_dir_all(fixed_paths::KEYS_DIR)
            .await
            .map_err(|e| CliError::Setup(format!("Failed to create keys dir: {e}")))?;

        // Initialize trusted_keys.json if missing
        let keys_file = std::path::Path::new(fixed_paths::KEYS_DIR).join("trusted_keys.json");
        if !keys_file.exists() {
            let trusted = serde_json::json!({
                "bootstrap": {
                    "key_id": "bootstrap",
                    "public_key": bootstrap_key,
                    "comment": "Bootstrap key",
                    "trusted_since": chrono::Utc::now().timestamp(),
                    "expires_at": null,
                    "algorithm": "minisign"
                }
            });
            fs::write(&keys_file, serde_json::to_string_pretty(&trusted).unwrap())
                .await
                .map_err(|e| CliError::Setup(format!("Failed to write trusted keys: {e}")))?;
        }

        Ok(())
    }

    /// Check permissions on system directories
    async fn check_permissions(&self) -> Result<(), CliError> {
        let paths_to_check = [
            fixed_paths::PREFIX,
            fixed_paths::STORE_DIR,
            fixed_paths::STATES_DIR,
            fixed_paths::LIVE_DIR,
        ];

        for path in &paths_to_check {
            let metadata = tokio::fs::metadata(path)
                .await
                .map_err(|e| CliError::Setup(format!("Cannot access {path}: {e}")))?;

            // Check if we can write to the directory
            if metadata.permissions().readonly() {
                return Err(CliError::Setup(format!("No write permission for {path}")));
            }
        }

        Ok(())
    }

    /// Initialize package store
    async fn init_store(&mut self) -> Result<(), CliError> {
        debug!("Initializing package store");
        let store_path = Path::new(fixed_paths::STORE_DIR);
        let store = PackageStore::new(store_path.to_path_buf());

        self.store = Some(store);
        Ok(())
    }

    /// Initialize state manager
    async fn init_state(&mut self) -> Result<(), CliError> {
        debug!("Initializing state manager");
        let state_path = Path::new(fixed_paths::PREFIX);
        let state = StateManager::new(state_path)
            .await
            .map_err(|e| CliError::Setup(format!("Failed to initialize state: {e}")))?;

        self.state = Some(state);
        Ok(())
    }

    /// Initialize index manager
    async fn init_index(&mut self) -> Result<(), CliError> {
        debug!("Initializing index manager");
        let cache_path = Path::new(fixed_paths::PREFIX);
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
                    .map_err(|e| CliError::Setup(format!("Failed to create empty index: {e}")))?;
                index
                    .load(Some(&json))
                    .await
                    .map_err(|e| CliError::Setup(format!("Failed to load empty index: {e}")))?;
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
            .map_err(|e| CliError::Setup(format!("Failed to create network client: {e}")))?;

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

    /// Initialize platform cache
    async fn init_platform_cache(&mut self) -> Result<(), CliError> {
        debug!("Initializing platform cache");

        // Initialize the platform manager's cache from persistent storage
        let platform_manager = sps2_platform::core::PlatformManager::instance();
        platform_manager
            .initialize_cache()
            .await
            .map_err(|e| CliError::Setup(format!("Failed to initialize platform cache: {e}")))?;

        debug!("Platform cache initialized successfully");
        Ok(())
    }

    /// Perform startup maintenance tasks
    async fn startup_maintenance(&mut self) -> Result<(), CliError> {
        debug!("Performing startup maintenance");

        // Check if garbage collection is needed (>7 days since last GC)
        let state = self.state.as_ref().unwrap();
        if self.should_run_startup_gc(state).await? {
            info!("Running startup garbage collection");

            // Clean up old states using configured retention count
            let cleaned_states = state
                .cleanup_old_states(self.config.state.retention_count)
                .await
                .map_err(|e| CliError::Setup(format!("Startup GC failed: {e}")))?;

            // Clean up orphaned packages
            let store = self.store.as_ref().unwrap();
            let cleaned_packages = store
                .garbage_collect()
                .map_err(|e| CliError::Setup(format!("Startup GC failed: {e}")))?;

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

    /// Clean up orphaned staging directories (only safe to remove)
    async fn clean_orphaned_staging(&self) -> Result<(), CliError> {
        let states_dir = Path::new(fixed_paths::STATES_DIR);
        if !states_dir.exists() {
            return Ok(());
        }

        let mut entries = tokio::fs::read_dir(states_dir)
            .await
            .map_err(|e| CliError::Setup(format!("Failed to read states directory: {e}")))?;

        let mut cleaned = 0;
        let state_manager = self.state.as_ref().unwrap();

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| CliError::Setup(format!("Failed to read directory entry: {e}")))?
        {
            let file_name = entry.file_name();
            if let Some(name) = file_name.to_str() {
                if name.starts_with("staging-") {
                    // Extract staging ID from directory name
                    if let Some(id_str) = name.strip_prefix("staging-") {
                        if let Ok(staging_id) = uuid::Uuid::parse_str(id_str) {
                            // Only remove if it's safe to do so
                            match state_manager.can_remove_staging(&staging_id).await {
                                Ok(true) => {
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
                                Ok(false) => {
                                    debug!("Staging directory {} is still in use, skipping", name);
                                }
                                Err(e) => {
                                    warn!("Failed to check if staging directory {} can be removed: {}", name, e);
                                }
                            }
                        }
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
            .map_err(|e| CliError::Setup(format!("Failed to read GC timestamp: {e}")))?;

        content
            .trim()
            .parse::<u64>()
            .map_err(|e| CliError::Setup(format!("Invalid GC timestamp format: {e}")))
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
            .map_err(|e| CliError::Setup(format!("Failed to write GC timestamp: {e}")))?;

        debug!("Updated GC timestamp: {}", now);
        Ok(())
    }

    /// Get the path to the GC timestamp file
    fn gc_timestamp_path(&self) -> PathBuf {
        Path::new(fixed_paths::LAST_GC_TIMESTAMP).to_path_buf()
    }

    /// Update GC timestamp - public static method for ops crate
    pub async fn update_gc_timestamp_static() -> Result<(), CliError> {
        let timestamp_path = std::path::Path::new(fixed_paths::LAST_GC_TIMESTAMP);
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
