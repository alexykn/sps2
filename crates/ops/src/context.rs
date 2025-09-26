//! Operations context for dependency injection

use sps2_builder::Builder;
use sps2_config::Config;
use sps2_events::{EventEmitter, EventSender};
use sps2_index::IndexManager;
use sps2_net::NetClient;
use sps2_resolver::Resolver;
use sps2_state::StateManager;
use sps2_store::PackageStore;
use std::cell::RefCell;
use std::fmt::Write;

/// Operations context providing access to all system components.
pub struct OpsCtx {
    pub store: PackageStore,
    pub state: StateManager,
    pub index: IndexManager,
    pub net: NetClient,
    pub resolver: Resolver,
    pub builder: Builder,
    pub tx: EventSender,
    pub config: Config,
    pub check_mode: bool,
    correlation_id: RefCell<Option<String>>,
}

impl EventEmitter for OpsCtx {
    fn event_sender(&self) -> Option<&EventSender> {
        Some(&self.tx)
    }

    fn enrich_event_meta(&self, _event: &sps2_events::AppEvent, meta: &mut sps2_events::EventMeta) {
        if let Some(correlation) = self.correlation_id.borrow().as_ref() {
            meta.correlation_id = Some(correlation.clone());
        }
        if self.check_mode {
            meta.labels
                .entry("check_mode".to_string())
                .or_insert_with(|| "true".to_string());
        }
    }
}

impl OpsCtx {
    #[must_use]
    pub fn push_correlation(&self, correlation: impl Into<String>) -> CorrelationGuard<'_> {
        let mut slot = self.correlation_id.borrow_mut();
        let previous = slot.replace(correlation.into());
        CorrelationGuard {
            ctx: self,
            previous,
        }
    }

    #[must_use]
    pub fn push_correlation_for_packages(
        &self,
        operation: &str,
        packages: &[String],
    ) -> CorrelationGuard<'_> {
        let mut identifier = operation.to_string();
        if !packages.is_empty() {
            identifier.push(':');
            let sample_len = packages.len().min(3);
            let sample = packages
                .iter()
                .take(sample_len)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(",");
            identifier.push_str(&sample);
            if packages.len() > sample_len {
                let _ = write!(&mut identifier, ",+{}", packages.len() - sample_len);
            }
        }
        self.push_correlation(identifier)
    }

    #[must_use]
    pub fn current_correlation(&self) -> Option<String> {
        self.correlation_id.borrow().clone()
    }
}

pub struct CorrelationGuard<'a> {
    ctx: &'a OpsCtx,
    previous: Option<String>,
}

impl Drop for CorrelationGuard<'_> {
    fn drop(&mut self) {
        *self.ctx.correlation_id.borrow_mut() = self.previous.take();
    }
}

pub struct OpsContextBuilder {
    store: Option<PackageStore>,
    state: Option<StateManager>,
    index: Option<IndexManager>,
    net: Option<NetClient>,
    resolver: Option<Resolver>,
    builder: Option<Builder>,
    tx: Option<EventSender>,
    config: Option<Config>,
    check_mode: Option<bool>,
}

impl OpsContextBuilder {
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
            check_mode: None,
        }
    }

    #[must_use]
    pub fn with_store(mut self, store: PackageStore) -> Self {
        self.store = Some(store);
        self
    }

    #[must_use]
    pub fn with_state(mut self, state: StateManager) -> Self {
        self.state = Some(state);
        self
    }

    #[must_use]
    pub fn with_index(mut self, index: IndexManager) -> Self {
        self.index = Some(index);
        self
    }

    #[must_use]
    pub fn with_net(mut self, net: NetClient) -> Self {
        self.net = Some(net);
        self
    }

    #[must_use]
    pub fn with_resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    #[must_use]
    pub fn with_builder(mut self, builder: Builder) -> Self {
        self.builder = Some(builder);
        self
    }

    #[must_use]
    pub fn with_event_sender(mut self, tx: EventSender) -> Self {
        self.tx = Some(tx);
        self
    }

    #[must_use]
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    #[must_use]
    pub fn with_check_mode(mut self, check_mode: bool) -> Self {
        self.check_mode = Some(check_mode);
        self
    }

    /// # Errors
    ///
    /// Returns an error if any required dependency is missing from the builder.
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
            check_mode: self.check_mode.unwrap_or(false),
            correlation_id: RefCell::new(None),
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
    async fn correlation_helpers_round_trip() {
        let temp_dir = TempDir::new().unwrap();
        let state_dir = temp_dir.path().join("state");
        let store_dir = temp_dir.path().join("store");

        tokio::fs::create_dir_all(&state_dir).await.unwrap();
        tokio::fs::create_dir_all(&store_dir).await.unwrap();

        let state = StateManager::new(&state_dir).await.unwrap();
        let store = PackageStore::new(store_dir.clone());
        let (tx, _rx) = sps2_events::channel();
        let config = Config::default();

        let index = IndexManager::new(&store_dir);
        let net = NetClient::new(sps2_net::NetConfig::default()).unwrap();
        let resolver = Resolver::with_events(index.clone(), tx.clone());
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

        assert!(ctx.current_correlation().is_none());
        {
            let _guard = ctx.push_correlation("install");
            assert_eq!(ctx.current_correlation(), Some("install".to_string()));
        }
        assert!(ctx.current_correlation().is_none());
    }
}
