//! Main installer implementation

use crate::{
    AtomicInstaller, InstallContext, InstallOperation, InstallResult, ParallelExecutor,
    UninstallContext, UninstallOperation, UpdateContext, UpdateOperation,
};
use spsv2_errors::{Error, InstallError};
use spsv2_events::EventSender;
use spsv2_resolver::Resolver;
use spsv2_state::StateManager;
use spsv2_store::PackageStore;
use uuid::Uuid;

/// Installer configuration
#[derive(Clone, Debug)]
pub struct InstallConfig {
    /// Maximum concurrent downloads
    pub max_concurrency: usize,
    /// Download timeout in seconds
    pub download_timeout: u64,
    /// Enable APFS optimizations
    pub enable_apfs: bool,
    /// State retention policy (number of states to keep)
    pub state_retention: usize,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            download_timeout: 300, // 5 minutes
            enable_apfs: cfg!(target_os = "macos"),
            state_retention: 10,
        }
    }
}

impl InstallConfig {
    /// Create config with custom concurrency
    pub fn with_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    /// Set download timeout
    pub fn with_timeout(mut self, timeout_seconds: u64) -> Self {
        self.download_timeout = timeout_seconds;
        self
    }

    /// Enable/disable APFS optimizations
    pub fn with_apfs(mut self, enable: bool) -> Self {
        self.enable_apfs = enable;
        self
    }

    /// Set state retention policy
    pub fn with_retention(mut self, count: usize) -> Self {
        self.state_retention = count;
        self
    }
}

/// Main installer for spsv2 packages
pub struct Installer {
    /// Configuration
    config: InstallConfig,
    /// Dependency resolver
    resolver: Resolver,
    /// State manager
    state_manager: StateManager,
    /// Package store
    store: PackageStore,
}

impl Installer {
    /// Create new installer
    pub fn new(
        config: InstallConfig,
        resolver: Resolver,
        state_manager: StateManager,
        store: PackageStore,
    ) -> Self {
        Self {
            config,
            resolver,
            state_manager,
            store,
        }
    }

    /// Install packages
    pub async fn install(&mut self, context: InstallContext) -> Result<InstallResult, Error> {
        // Validate context
        self.validate_install_context(&context)?;

        // Create install operation
        let mut operation = InstallOperation::new(
            self.resolver.clone(),
            self.state_manager.clone(),
            self.store.clone(),
        )?;

        // Execute installation
        let result = operation.execute(context).await?;

        // Trigger garbage collection
        self.cleanup_old_states().await?;

        Ok(result)
    }

    /// Uninstall packages
    pub async fn uninstall(&mut self, context: UninstallContext) -> Result<InstallResult, Error> {
        // Validate context
        self.validate_uninstall_context(&context)?;

        // Create uninstall operation
        let mut operation = UninstallOperation::new(self.state_manager.clone(), self.store.clone());

        // Execute uninstallation
        let result = operation.execute(context).await?;

        // Trigger garbage collection
        self.cleanup_old_states().await?;

        Ok(result)
    }

    /// Update packages
    pub async fn update(&mut self, context: UpdateContext) -> Result<InstallResult, Error> {
        // Validate context
        self.validate_update_context(&context)?;

        // Create update operation
        let mut operation = UpdateOperation::new(
            self.resolver.clone(),
            self.state_manager.clone(),
            self.store.clone(),
        )?;

        // Execute update
        let result = operation.execute(context).await?;

        // Trigger garbage collection
        self.cleanup_old_states().await?;

        Ok(result)
    }

    /// Rollback to a previous state
    pub async fn rollback(&mut self, target_state_id: Uuid) -> Result<(), Error> {
        // Validate target state exists
        if !self.state_manager.state_exists(&target_state_id).await? {
            return Err(InstallError::StateNotFound {
                state_id: target_state_id.to_string(),
            }
            .into());
        }

        // Create atomic installer for rollback
        let mut atomic_installer =
            AtomicInstaller::new(self.state_manager.clone(), self.store.clone());

        // Perform rollback
        atomic_installer.rollback(target_state_id).await?;

        Ok(())
    }

    /// List available states for rollback
    pub async fn list_states(&self) -> Result<Vec<StateInfo>, Error> {
        let states = self.state_manager.list_states().await?;

        let mut state_infos = Vec::new();
        for state_id in states {
            let packages = self.state_manager.get_state_packages(&state_id).await?;

            state_infos.push(StateInfo {
                id: state_id,
                timestamp: chrono::Utc::now(), // Placeholder - would need to fetch from state table
                parent_id: None, // Placeholder - would need to fetch from state table
                package_count: packages.len(),
                packages: packages.into_iter().take(5)
                    .map(|name| spsv2_types::PackageId::new(name, spsv2_types::Version::new(1, 0, 0)))
                    .collect(), // First 5 packages
            });
        }

        Ok(state_infos)
    }

    /// Get current state information
    pub async fn current_state(&self) -> Result<StateInfo, Error> {
        let current_id = self
            .state_manager
            .get_current_state_id()
            .await?;

        let states = self.list_states().await?;
        states
            .into_iter()
            .find(|state| state.id == current_id)
            .ok_or_else(|| {
                InstallError::StateNotFound {
                    state_id: current_id.to_string(),
                }
                .into()
            })
    }

    /// Cleanup old states according to retention policy
    async fn cleanup_old_states(&self) -> Result<(), Error> {
        self.state_manager
            .cleanup_old_states(self.config.state_retention)
            .await?;
        self.store.garbage_collect().await?;
        Ok(())
    }

    /// Validate install context
    fn validate_install_context(&self, context: &InstallContext) -> Result<(), Error> {
        if context.packages.is_empty() && context.local_files.is_empty() {
            return Err(InstallError::NoPackagesSpecified.into());
        }

        // Validate local file paths exist
        for path in &context.local_files {
            if !path.exists() {
                return Err(InstallError::LocalPackageNotFound {
                    path: path.display().to_string(),
                }
                .into());
            }

            if !path.extension().map_or(false, |ext| ext == "sp") {
                return Err(InstallError::InvalidPackageFile {
                    path: path.display().to_string(),
                    message: "file must have .sp extension".to_string(),
                }
                .into());
            }
        }

        Ok(())
    }

    /// Validate uninstall context
    fn validate_uninstall_context(&self, context: &UninstallContext) -> Result<(), Error> {
        if context.packages.is_empty() {
            return Err(InstallError::NoPackagesSpecified.into());
        }

        Ok(())
    }

    /// Validate update context
    fn validate_update_context(&self, _context: &UpdateContext) -> Result<(), Error> {
        // Update context is always valid (empty packages means update all)
        Ok(())
    }
}

/// State information for listing
#[derive(Debug, Clone)]
pub struct StateInfo {
    /// State ID
    pub id: Uuid,
    /// Creation timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Parent state ID
    pub parent_id: Option<Uuid>,
    /// Number of packages in this state
    pub package_count: usize,
    /// Sample of packages (for display)
    pub packages: Vec<spsv2_types::PackageId>,
}

impl StateInfo {
    /// Check if this is the root state
    pub fn is_root(&self) -> bool {
        self.parent_id.is_none()
    }

    /// Get age of this state
    pub fn age(&self) -> chrono::Duration {
        chrono::Utc::now() - self.timestamp
    }

    /// Format package list for display
    pub fn package_summary(&self) -> String {
        if self.packages.is_empty() {
            "No packages".to_string()
        } else if self.packages.len() <= 3 {
            self.packages
                .iter()
                .map(|pkg| format!("{}-{}", pkg.name, pkg.version))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            let first_three: Vec<String> = self
                .packages
                .iter()
                .take(3)
                .map(|pkg| format!("{}-{}", pkg.name, pkg.version))
                .collect();
            format!(
                "{} and {} more",
                first_three.join(", "),
                self.package_count - 3
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spsv2_index::{Index, IndexManager};
    use spsv2_types::PackageSpec;
    use tempfile::tempdir;

    async fn create_test_installer() -> Installer {
        let temp = tempdir().unwrap();

        // Create index manager with empty index
        let mut index_manager = IndexManager::new(temp.path());
        let index = Index::new();
        let json = index.to_json().unwrap();
        index_manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(index_manager);
        let state_manager = StateManager::new(temp.path()).await.unwrap();
        let store = PackageStore::new(temp.path()).await.unwrap();
        let config = InstallConfig::default();

        Installer::new(config, resolver, state_manager, store)
    }

    #[test]
    fn test_install_config() {
        let config = InstallConfig::default();
        assert_eq!(config.max_concurrency, 4);
        assert_eq!(config.download_timeout, 300);
        assert_eq!(config.state_retention, 10);

        let custom_config = InstallConfig::default()
            .with_concurrency(8)
            .with_timeout(600)
            .with_retention(20);

        assert_eq!(custom_config.max_concurrency, 8);
        assert_eq!(custom_config.download_timeout, 600);
        assert_eq!(custom_config.state_retention, 20);
    }

    #[tokio::test]
    async fn test_installer_creation() {
        let installer = create_test_installer().await;
        assert_eq!(installer.config.max_concurrency, 4);
    }

    #[tokio::test]
    async fn test_install_context_validation() {
        let installer = create_test_installer().await;

        // Empty context should fail
        let empty_context = InstallContext::new();
        assert!(installer.validate_install_context(&empty_context).is_err());

        // Context with packages should pass
        let valid_context =
            InstallContext::new().add_package(PackageSpec::parse("curl>=8.0.0").unwrap());
        assert!(installer.validate_install_context(&valid_context).is_ok());
    }

    #[tokio::test]
    async fn test_uninstall_context_validation() {
        let installer = create_test_installer().await;

        // Empty context should fail
        let empty_context = UninstallContext::new();
        assert!(installer
            .validate_uninstall_context(&empty_context)
            .is_err());

        // Context with packages should pass
        let valid_context = UninstallContext::new().add_package("curl".to_string());
        assert!(installer.validate_uninstall_context(&valid_context).is_ok());
    }

    #[tokio::test]
    async fn test_update_context_validation() {
        let installer = create_test_installer().await;

        // All update contexts are valid (empty means update all)
        let empty_context = UpdateContext::new();
        assert!(installer.validate_update_context(&empty_context).is_ok());

        let context_with_packages = UpdateContext::new().add_package("curl".to_string());
        assert!(installer
            .validate_update_context(&context_with_packages)
            .is_ok());
    }

    #[test]
    fn test_state_info() {
        let state_id = Uuid::new_v4();
        let timestamp = chrono::Utc::now();

        let state_info = StateInfo {
            id: state_id,
            timestamp,
            parent_id: None,
            package_count: 0,
            packages: Vec::new(),
        };

        assert!(state_info.is_root());
        assert_eq!(state_info.package_summary(), "No packages");

        // Test age calculation (should be very recent)
        let age = state_info.age();
        assert!(age.num_seconds() < 5);
    }
}
