//! Operations context for dependency injection

use sps2_builder::Builder;
use sps2_config::Config;
use sps2_errors::Error;
use sps2_events::{Event, EventSender};
use sps2_guard::{StateVerificationGuard, VerificationLevel};
use sps2_index::IndexManager;
use sps2_net::NetClient;
use sps2_resolver::Resolver;
use sps2_state::StateManager;
use sps2_store::PackageStore;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;

/// Operations context providing access to all system components
pub struct OpsCtx {
    /// Package store
    pub store: PackageStore,
    /// State manager
    pub state: StateManager,
    /// Index manager
    pub index: IndexManager,
    /// Network client
    pub net: NetClient,
    /// Dependency resolver
    pub resolver: Resolver,
    /// Package builder
    pub builder: Builder,
    /// Event sender for progress reporting
    pub tx: EventSender,
    /// System configuration
    pub config: Config,
    /// State verification guard (optional)
    pub guard: RefCell<Option<StateVerificationGuard>>,
}

impl OpsCtx {
    // No public constructor - use OpsContextBuilder instead

    /// Run state verification if guard is enabled
    ///
    /// # Errors
    ///
    /// Returns an error if verification fails and `fail_on_discrepancy` is true
    pub async fn verify_state(&self) -> Result<(), Error> {
        // Take the guard out of the RefCell to avoid holding borrow across await
        let guard_option = self.guard.borrow_mut().take();

        if let Some(mut guard) = guard_option {
            let result = if self.config.verification.auto_heal {
                guard.verify_and_heal(&self.config).await?
            } else {
                guard.verify_only().await?
            };

            // Put the guard back before checking result
            *self.guard.borrow_mut() = Some(guard);

            if !result.is_valid && self.config.verification.fail_on_discrepancy {
                return Err(sps2_errors::OpsError::VerificationFailed {
                    discrepancies: result.discrepancies.len(),
                    state_id: result.state_id.to_string(),
                }
                .into());
            }
        }
        Ok(())
    }

    /// Check if verification is enabled
    #[must_use]
    pub fn is_verification_enabled(&self) -> bool {
        self.guard.borrow().is_some() && self.config.verification.enabled
    }

    /// Initialize the state verification guard if enabled in config
    ///
    /// # Errors
    ///
    /// Returns an error if guard initialization fails.
    pub fn initialize_guard(&mut self) -> Result<(), Error> {
        // Check if verification is enabled in config
        if !self.config.verification.enabled {
            return Ok(());
        }

        // Parse verification level from config
        let level = match self.config.verification.level.as_str() {
            "quick" => VerificationLevel::Quick,
            "full" => VerificationLevel::Full,
            _ => VerificationLevel::Standard,
        };

        // Build the guard
        let guard = StateVerificationGuard::builder()
            .with_state_manager(self.state.clone())
            .with_store(self.store.clone())
            .with_event_sender(self.tx.clone())
            .with_level(level)
            .build()?;

        *self.guard.borrow_mut() = Some(guard);
        Ok(())
    }

    /// Execute an operation with automatic state verification
    ///
    /// This wrapper runs state verification before and after the operation
    /// if verification is enabled.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Pre-operation verification fails (when `fail_on_discrepancy` is true)
    /// - The operation itself fails
    /// - Post-operation verification fails (when `fail_on_discrepancy` is true)
    pub async fn with_verification<F, Fut, T>(
        &self,
        operation_name: &str,
        operation: F,
    ) -> Result<T, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, Error>>,
    {
        // Emit operation start event
        let _ = self.tx.send(Event::DebugLog {
            message: format!("Starting operation: {operation_name}"),
            context: HashMap::default(),
        });

        // Run pre-operation verification if enabled
        if self.is_verification_enabled() {
            let _ = self.tx.send(Event::DebugLog {
                message: format!("Running pre-operation verification for {operation_name}"),
                context: HashMap::default(),
            });
            self.verify_state().await?;
        }

        // Execute the operation
        let result = operation().await?;

        // Run post-operation verification if enabled
        if self.is_verification_enabled() {
            let _ = self.tx.send(Event::DebugLog {
                message: format!("Running post-operation verification for {operation_name}"),
                context: HashMap::default(),
            });
            self.verify_state().await?;
        }

        // Emit operation complete event
        let _ = self.tx.send(Event::DebugLog {
            message: format!("Operation completed: {operation_name}"),
            context: HashMap::default(),
        });

        Ok(result)
    }
}

// Example usage in an operation:
// ```rust
// pub async fn install_with_guard(ctx: &OpsCtx, packages: &[String]) -> Result<InstallReport, Error> {
//     ctx.with_verification("install", || async {
//         install(ctx, packages).await
//     }).await
// }
// ```

/// Builder for operations context
pub struct OpsContextBuilder {
    store: Option<PackageStore>,
    state: Option<StateManager>,
    index: Option<IndexManager>,
    net: Option<NetClient>,
    resolver: Option<Resolver>,
    builder: Option<Builder>,
    tx: Option<EventSender>,
    config: Option<Config>,
}

impl OpsContextBuilder {
    /// Create new context builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: None,
            state: None,
            index: None,
            net: None,
            resolver: None,
            builder: None,
            tx: None,
            config: None,
        }
    }

    /// Set package store
    #[must_use]
    pub fn with_store(mut self, store: PackageStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Set state manager
    #[must_use]
    pub fn with_state(mut self, state: StateManager) -> Self {
        self.state = Some(state);
        self
    }

    /// Set index manager
    #[must_use]
    pub fn with_index(mut self, index: IndexManager) -> Self {
        self.index = Some(index);
        self
    }

    /// Set network client
    #[must_use]
    pub fn with_net(mut self, net: NetClient) -> Self {
        self.net = Some(net);
        self
    }

    /// Set resolver
    #[must_use]
    pub fn with_resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Set builder
    #[must_use]
    pub fn with_builder(mut self, builder: Builder) -> Self {
        self.builder = Some(builder);
        self
    }

    /// Set event sender
    #[must_use]
    pub fn with_event_sender(mut self, tx: EventSender) -> Self {
        self.tx = Some(tx);
        self
    }

    /// Set configuration
    #[must_use]
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    /// Build the context
    ///
    /// # Errors
    ///
    /// Returns an error if any required component is missing.
    pub fn build(self) -> Result<OpsCtx, sps2_errors::Error> {
        let store = self
            .store
            .ok_or_else(|| sps2_errors::OpsError::MissingComponent {
                component: "store".to_string(),
            })?;

        let state = self
            .state
            .ok_or_else(|| sps2_errors::OpsError::MissingComponent {
                component: "state".to_string(),
            })?;

        let index = self
            .index
            .ok_or_else(|| sps2_errors::OpsError::MissingComponent {
                component: "index".to_string(),
            })?;

        let net = self
            .net
            .ok_or_else(|| sps2_errors::OpsError::MissingComponent {
                component: "net".to_string(),
            })?;

        let resolver = self
            .resolver
            .ok_or_else(|| sps2_errors::OpsError::MissingComponent {
                component: "resolver".to_string(),
            })?;

        let builder = self
            .builder
            .ok_or_else(|| sps2_errors::OpsError::MissingComponent {
                component: "builder".to_string(),
            })?;

        let tx = self
            .tx
            .ok_or_else(|| sps2_errors::OpsError::MissingComponent {
                component: "event_sender".to_string(),
            })?;

        let config = self
            .config
            .ok_or_else(|| sps2_errors::OpsError::MissingComponent {
                component: "config".to_string(),
            })?;

        Ok(OpsCtx {
            store,
            state,
            index,
            net,
            resolver,
            builder,
            tx,
            config,
            guard: RefCell::new(None), // Guard will be initialized separately
        })
    }
}

impl Default for OpsContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_verification_disabled_by_default() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");

        tokio::fs::create_dir_all(&state_dir).await.unwrap();
        tokio::fs::create_dir_all(&store_dir).await.unwrap();

        let state = StateManager::new(&state_dir).await.unwrap();
        let store = PackageStore::new(store_dir.clone());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let config = Config::default();

        let index = IndexManager::new(&store_dir);
        let net = NetClient::new(sps2_net::NetConfig::default()).unwrap();
        let resolver = Resolver::new(index.clone());
        let builder = Builder::new();

        let ctx = OpsContextBuilder::new()
            .with_state(state)
            .with_store(store)
            .with_event_sender(tx)
            .with_config(config)
            .with_index(index)
            .with_net(net)
            .with_resolver(resolver)
            .with_builder(builder)
            .build()
            .unwrap();

        assert!(!ctx.is_verification_enabled());
        assert!(ctx.guard.borrow().is_none());
    }

    #[tokio::test]
    async fn test_verification_can_be_enabled() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");

        tokio::fs::create_dir_all(&state_dir).await.unwrap();
        tokio::fs::create_dir_all(&store_dir).await.unwrap();

        let state = StateManager::new(&state_dir).await.unwrap();
        let store = PackageStore::new(store_dir.clone());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let mut config = Config::default();
        config.verification.enabled = true;

        let index = IndexManager::new(&store_dir);
        let net = NetClient::new(sps2_net::NetConfig::default()).unwrap();
        let resolver = Resolver::new(index.clone());
        let builder = Builder::new();

        let mut ctx = OpsContextBuilder::new()
            .with_state(state)
            .with_store(store)
            .with_event_sender(tx)
            .with_config(config)
            .with_index(index)
            .with_net(net)
            .with_resolver(resolver)
            .with_builder(builder)
            .build()
            .unwrap();

        ctx.initialize_guard().unwrap();

        assert!(ctx.is_verification_enabled());
        assert!(ctx.guard.borrow().is_some());
    }

    #[tokio::test]
    async fn test_with_verification_wrapper() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");

        tokio::fs::create_dir_all(&state_dir).await.unwrap();
        tokio::fs::create_dir_all(&store_dir).await.unwrap();

        let state = StateManager::new(&state_dir).await.unwrap();
        let store = PackageStore::new(store_dir.clone());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let config = Config::default();

        let index = IndexManager::new(&store_dir);
        let net = NetClient::new(sps2_net::NetConfig::default()).unwrap();
        let resolver = Resolver::new(index.clone());
        let builder = Builder::new();

        let ctx = OpsContextBuilder::new()
            .with_state(state)
            .with_store(store)
            .with_event_sender(tx)
            .with_config(config)
            .with_index(index)
            .with_net(net)
            .with_resolver(resolver)
            .with_builder(builder)
            .build()
            .unwrap();

        // Test that operations work even without verification enabled
        let result = ctx
            .with_verification("test_op", || async { Ok("success") })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }
}
