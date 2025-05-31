//! Operations context for dependency injection

use sps2_builder::Builder;
use sps2_events::EventSender;
use sps2_index::IndexManager;
use sps2_net::NetClient;
use sps2_resolver::Resolver;
use sps2_state::StateManager;
use sps2_store::PackageStore;

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
}

impl OpsCtx {
    /// Create new operations context
    #[must_use]
    pub fn new(
        store: PackageStore,
        state: StateManager,
        index: IndexManager,
        net: NetClient,
        resolver: Resolver,
        builder: Builder,
        tx: EventSender,
    ) -> Self {
        Self {
            store,
            state,
            index,
            net,
            resolver,
            builder,
            tx,
        }
    }
}

/// Builder for operations context
pub struct OpsContextBuilder {
    store: Option<PackageStore>,
    state: Option<StateManager>,
    index: Option<IndexManager>,
    net: Option<NetClient>,
    resolver: Option<Resolver>,
    builder: Option<Builder>,
    tx: Option<EventSender>,
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

        Ok(OpsCtx::new(store, state, index, net, resolver, builder, tx))
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
    use tempfile::tempdir;

    async fn create_test_components() -> (
        PackageStore,
        StateManager,
        IndexManager,
        NetClient,
        Resolver,
        Builder,
        EventSender,
    ) {
        let temp = tempdir().unwrap();

        let store = PackageStore::new(temp.path().to_path_buf());
        let state = StateManager::new(temp.path()).await.unwrap();
        let index = IndexManager::new(temp.path());
        let net = NetClient::with_defaults().unwrap();
        let resolver = Resolver::new(index.clone());
        let builder = Builder::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        (store, state, index, net, resolver, builder, tx)
    }

    #[tokio::test]
    async fn test_context_builder() {
        let (store, state, index, net, resolver, builder, tx) = create_test_components().await;

        let _ctx = OpsContextBuilder::new()
            .with_store(store)
            .with_state(state)
            .with_index(index)
            .with_net(net)
            .with_resolver(resolver)
            .with_builder(builder)
            .with_event_sender(tx)
            .build()
            .unwrap();

        // Verify context was built successfully
        // Context was built successfully, no further assertions needed
    }

    #[test]
    fn test_incomplete_context_builder() {
        let builder = OpsContextBuilder::new();
        let result = builder.build();

        assert!(result.is_err());
        // Should fail due to missing components
    }
}
