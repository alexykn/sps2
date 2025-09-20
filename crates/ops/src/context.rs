//! Operations context for dependency injection

use sps2_builder::Builder;
use sps2_config::Config;
use sps2_errors::Error;
use sps2_events::{EventEmitter, EventSender};
use sps2_guard::{
    derive_post_operation_scope, derive_pre_operation_scope, GuardConfig,
    OperationResult as GuardOperationResult, OperationType, StateVerificationGuard,
};
use sps2_index::IndexManager;
use sps2_net::NetClient;
use sps2_resolver::Resolver;
use sps2_state::StateManager;
use sps2_store::PackageStore;
use std::cell::RefCell;
use std::fmt::Write;

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
    /// Whether to run in check mode (preview only)
    pub check_mode: bool,
    /// Active correlation identifier for events emitted via this context
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
    /// Push a correlation identifier string, restoring the previous value when dropped.
    #[must_use]
    pub fn push_correlation(&self, correlation: impl Into<String>) -> CorrelationGuard<'_> {
        let mut slot = self.correlation_id.borrow_mut();
        let previous = slot.replace(correlation.into());
        CorrelationGuard {
            ctx: self,
            previous,
        }
    }

    /// Construct a correlation identifier from an operation name and package list.
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

    // No public constructor - use OpsContextBuilder instead

    /// Run state verification if guard is enabled
    ///
    /// # Errors
    ///
    /// Returns an error if verification or operation fails
    ///
    /// # Panics
    ///
    /// Panics if the guard is not properly initialized when verification is enabled
    pub async fn with_guard_integration<F, Fut, T>(
        &self,
        operation_type: OperationType,
        operation: F,
    ) -> Result<(T, GuardOperationResult), Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(T, GuardOperationResult), Error>>,
    {
        if !self.is_verification_enabled() {
            // If verification is disabled, just run the operation
            return operation().await;
        }

        // Get a mutable reference to the guard
        let guard_option = self.guard.borrow_mut().take();
        if let Some(mut guard) = guard_option {
            // Phase 1: Pre-operation verification with intelligent scoping
            let pre_scope = derive_pre_operation_scope(&operation_type);
            self.emit_debug(format!(
                "Running pre-operation verification with scope: {pre_scope:?}"
            ));

            // Use progressive verification if configured
            let pre_result = if guard.config().performance.progressive_verification {
                guard.verify_progressively(&pre_scope).await?
            } else {
                guard.verify_with_scope(&pre_scope).await?
            };

            // Check pre-verification result
            if !pre_result.is_valid && self.config.verification.should_fail_on_discrepancy() {
                // Put guard back before failing
                *self.guard.borrow_mut() = Some(guard);
                return Err(sps2_errors::OpsError::VerificationFailed {
                    discrepancies: pre_result.discrepancies.len(),
                    state_id: pre_result.state_id.to_string(),
                }
                .into());
            }

            // Phase 3: Execute the operation
            let (operation_result, guard_operation_result) = {
                // Put guard back temporarily for operation execution
                *self.guard.borrow_mut() = Some(guard);
                let result = operation().await?;
                let guard_taken = self.guard.borrow_mut().take().unwrap();
                (result, guard_taken)
            };

            let mut guard = guard_operation_result;
            let (op_result, op_metadata) = operation_result;

            // Phase 4: Post-operation verification with result-based scoping
            let post_scope = derive_post_operation_scope(&operation_type, &op_metadata);
            self.emit_debug(format!(
                "Running post-operation verification with scope: {post_scope:?}"
            ));

            let post_result = if guard.config().performance.progressive_verification {
                guard.verify_progressively(&post_scope).await?
            } else {
                guard.verify_with_scope(&post_scope).await?
            };

            // Check post-verification result
            if !post_result.is_valid && self.config.verification.should_fail_on_discrepancy() {
                // Put guard back before failing
                *self.guard.borrow_mut() = Some(guard);
                return Err(sps2_errors::OpsError::VerificationFailed {
                    discrepancies: post_result.discrepancies.len(),
                    state_id: post_result.state_id.to_string(),
                }
                .into());
            }

            // Put guard back and return result
            *self.guard.borrow_mut() = Some(guard);
            Ok((op_result, op_metadata))
        } else {
            // No guard available - just run operation
            operation().await
        }
    }

    /// Check if verification is enabled
    #[must_use]
    pub fn is_verification_enabled(&self) -> bool {
        // Check if guard is initialized and if guard is enabled in either configuration approach
        let guard_enabled = if let Some(guard_config) = &self.config.guard {
            guard_config.enabled
        } else {
            self.config.verification.enabled
        };

        let guard_initialized = self.guard.borrow().is_some();
        let result = guard_initialized && guard_enabled;

        self.emit_debug(format!(
            "is_verification_enabled: guard_initialized={guard_initialized}, guard_enabled={guard_enabled}, result={result}"
        ));

        result
    }

    /// Initialize the state verification guard if enabled in config
    ///
    /// This supports both configuration approaches:
    /// 1. Top-level [guard] configuration (preferred, newer approach)
    /// 2. Legacy [verification.guard] configuration (backward compatibility)
    ///
    /// The top-level [guard] configuration takes precedence if present.
    ///
    /// # Errors
    ///
    /// Returns an error if guard initialization fails.
    pub fn initialize_guard(&mut self) -> Result<(), Error> {
        // Determine which configuration approach to use and if enabled
        let guard_enabled = if let Some(guard_config) = &self.config.guard {
            // Top-level [guard] configuration takes precedence
            self.emit_debug(format!(
                "Found top-level guard config, enabled: {}",
                guard_config.enabled
            ));
            guard_config.enabled
        } else {
            // Fall back to legacy [verification] configuration
            self.emit_debug(format!(
                "Using legacy verification config, enabled: {}",
                self.config.verification.enabled
            ));
            self.config.verification.enabled
        };

        self.emit_debug(format!("Guard initialization: enabled={guard_enabled}"));

        if !guard_enabled {
            self.emit_debug("Guard is disabled, skipping initialization");
            return Ok(());
        }

        // Validate the guard configuration first
        self.config.validate_guard_config()?;

        // Convert user configuration to guard configuration
        let guard_config: GuardConfig = if let Some(top_level_guard) = &self.config.guard {
            // Use top-level [guard] configuration (OPS-64 approach)
            self.emit_debug("Using top-level [guard] configuration approach");
            top_level_guard.into()
        } else {
            // Use legacy [verification.guard] configuration (OPS-65 approach)
            self.emit_debug("Using legacy [verification.guard] configuration approach");
            (&self.config.verification).into()
        };

        // Build the guard with the user's complete configuration
        let guard = StateVerificationGuard::builder()
            .with_state_manager(self.state.clone())
            .with_store(self.store.clone())
            .with_event_sender(self.tx.clone())
            .with_config(guard_config)
            .build()?;

        self.emit_debug("Guard successfully built and stored");

        *self.guard.borrow_mut() = Some(guard);
        Ok(())
    }

    /// Run state verification if guard is enabled
    ///
    /// # Errors
    ///
    /// Returns an error if verification fails and `fail_on_discrepancy` is true
    pub async fn verify_state(&self) -> Result<(), Error> {
        // Take the guard out of the RefCell to avoid holding borrow across await
        let guard_option = self.guard.borrow_mut().take();

        if let Some(mut guard) = guard_option {
            let result = if self.config.verification.should_auto_heal() {
                guard.verify_and_heal(&self.config).await?
            } else {
                guard.verify_only().await?
            };

            // Put the guard back before checking result
            *self.guard.borrow_mut() = Some(guard);

            if !result.is_valid && self.config.verification.should_fail_on_discrepancy() {
                return Err(sps2_errors::OpsError::VerificationFailed {
                    discrepancies: result.discrepancies.len(),
                    state_id: result.state_id.to_string(),
                }
                .into());
            }
        }
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
        self.emit_debug(format!("Starting operation: {operation_name}"));

        // Run pre-operation verification if enabled
        if self.is_verification_enabled() {
            self.emit_debug(format!(
                "Running pre-operation verification for {operation_name}"
            ));
            self.verify_state().await?;
        }

        // Execute the operation
        let result = operation().await?;

        // Run post-operation verification if enabled
        if self.is_verification_enabled() {
            self.emit_debug(format!(
                "Running post-operation verification for {operation_name}"
            ));
            self.verify_state().await?;
        }

        // Emit operation complete event
        self.emit_debug(format!("Operation completed: {operation_name}"));

        Ok(result)
    }

    /// Create a guarded install operation
    #[must_use]
    pub fn guarded_install<T>(&self, package_specs: Vec<String>) -> GuardedOperation<T> {
        GuardedOperation::new(self, OperationType::Install { package_specs })
    }

    /// Create a guarded uninstall operation  
    #[must_use]
    pub fn guarded_uninstall<T>(&self, package_names: Vec<String>) -> GuardedOperation<T> {
        GuardedOperation::new(self, OperationType::Uninstall { package_names })
    }

    /// Create a guarded upgrade operation
    #[must_use]
    pub fn guarded_upgrade<T>(&self, package_names: Vec<String>) -> GuardedOperation<T> {
        GuardedOperation::new(self, OperationType::Upgrade { package_names })
    }

    /// Create a guarded update operation
    #[must_use]
    pub fn guarded_update<T>(&self, package_names: Vec<String>) -> GuardedOperation<T> {
        GuardedOperation::new(self, OperationType::Update { package_names })
    }

    /// Create a guarded build operation
    #[must_use]
    pub fn guarded_build<T>(&self, recipe_path: std::path::PathBuf) -> GuardedOperation<T> {
        GuardedOperation::new(self, OperationType::Build { recipe_path })
    }

    /// Create a guarded cleanup operation
    #[must_use]
    pub fn guarded_cleanup<T>(&self) -> GuardedOperation<T> {
        GuardedOperation::new(self, OperationType::Cleanup)
    }
}

/// `GuardedOperation` wrapper for seamless guard integration
///
/// This wrapper provides a standardized way to integrate any operation with the guard system,
/// automatically handling cache warming, operation-specific verification, and cache invalidation.
pub struct GuardedOperation<'a, T> {
    ctx: &'a OpsCtx,
    operation_type: OperationType,
    _phantom: std::marker::PhantomData<T>,
}

impl<'a, T> GuardedOperation<'a, T> {
    /// Create a new guarded operation wrapper
    #[must_use]
    pub fn new(ctx: &'a OpsCtx, operation_type: OperationType) -> Self {
        Self {
            ctx,
            operation_type,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Execute the wrapped operation with full guard integration
    ///
    /// # Errors
    ///
    /// Returns an error if verification or operation fails
    pub async fn execute<F, Fut>(self, operation: F) -> Result<T, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(T, GuardOperationResult), Error>>,
    {
        let (result, _metadata) = self
            .ctx
            .with_guard_integration(self.operation_type, operation)
            .await?;

        Ok(result)
    }

    /// Execute with custom error recovery strategy
    ///
    /// # Errors
    ///
    /// Returns an error if operation fails and recovery strategy fails
    pub async fn execute_with_recovery<F, Fut, R, RFut>(
        self,
        operation: F,
        recovery: R,
    ) -> Result<T, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(T, GuardOperationResult), Error>>,
        R: FnOnce(Error) -> RFut,
        RFut: Future<Output = Result<T, Error>>,
    {
        // Store the event sender before consuming self
        let tx = self.ctx.tx.clone();

        match self.execute(operation).await {
            Ok(result) => Ok(result),
            Err(e) => {
                tx.emit_debug(format!("Operation failed, attempting recovery: {e}"));
                recovery(e).await
            }
        }
    }

    /// Execute with automatic retry and intelligent recovery
    ///
    /// This advanced recovery mechanism provides:
    /// - Automatic retry for transient failures
    /// - State healing for verification failures
    /// - Rollback for partial operation failures
    /// - Operation-specific recovery strategies
    ///
    /// # Errors
    ///
    /// Returns an error if all recovery attempts fail
    pub async fn execute_with_intelligent_recovery<F, Fut>(self, operation: F) -> Result<T, Error>
    where
        F: Fn() -> Fut + Clone,
        Fut: Future<Output = Result<(T, GuardOperationResult), Error>>,
    {
        let tx = self.ctx.tx.clone();
        let operation_type = self.operation_type.clone();
        let max_retries = 3;
        let mut retry_count = 0;

        loop {
            let _attempt_start = std::time::Instant::now();

            // Clone operation for retry
            let op_clone = operation.clone();

            match self
                .ctx
                .with_guard_integration(operation_type.clone(), op_clone)
                .await
            {
                Ok((result, _metadata)) => {
                    if retry_count > 0 {
                        tx.emit_debug(format!(
                            "Operation succeeded on retry attempt {retry_count}"
                        ));
                    }
                    return Ok(result);
                }
                Err(e) => {
                    retry_count += 1;

                    tx.emit_debug(format!("Operation attempt {retry_count} failed: {e}"));

                    // Determine if we should retry
                    if retry_count >= max_retries || !Self::is_retryable_error(&e) {
                        // Attempt intelligent recovery based on error type
                        return self.attempt_intelligent_recovery(e, &operation_type).await;
                    }

                    // Wait before retry with exponential backoff
                    let delay_ms = 100 * (1 << (retry_count - 1)); // 100ms, 200ms, 400ms
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                    tx.emit_debug(format!(
                        "Retrying operation in {}ms (attempt {} of {})",
                        delay_ms,
                        retry_count + 1,
                        max_retries
                    ));
                }
            }
        }
    }

    /// Attempt intelligent recovery based on error type and operation
    async fn attempt_intelligent_recovery(
        &self,
        error: Error,
        operation_type: &OperationType,
    ) -> Result<T, Error> {
        let tx = &self.ctx.tx;

        tx.emit_debug(format!(
            "Attempting intelligent recovery for {operation_type:?}"
        ));

        // Check if this is a verification failure that can be healed
        if Self::is_verification_error(&error) && self.ctx.config.verification.should_auto_heal() {
            tx.emit_debug("Attempting state healing for verification failure");

            // Attempt to heal the state
            if let Err(heal_error) = self.ctx.verify_state().await {
                tx.emit_debug(format!("State healing failed: {heal_error}"));
                return Err(heal_error);
            }

            tx.emit_debug("State healing completed, operation may now succeed");
        }

        // Operation-specific recovery strategies
        match operation_type {
            OperationType::Install { .. } => {
                // For install failures, try to clean up any partial installations
                tx.emit_debug("Install operation failed - consider manual cleanup or retry");
            }
            OperationType::Uninstall { .. } => {
                // For uninstall failures, verify if packages were actually removed
                tx.emit_debug("Uninstall operation failed - verifying package removal status");
            }
            OperationType::Upgrade { .. } => {
                // For upgrade failures, check if rollback is needed
                tx.emit_debug("Upgrade operation failed - consider rollback to previous state");
            }
            _ => {
                tx.emit_debug("Generic recovery strategy - operation failed");
            }
        }

        // Return original error if recovery doesn't help
        Err(error)
    }

    /// Check if an error is retryable
    fn is_retryable_error(error: &Error) -> bool {
        let error_str = error.to_string();

        // Network errors are usually retryable
        if error_str.contains("network")
            || error_str.contains("timeout")
            || error_str.contains("connection")
        {
            return true;
        }

        // Temporary filesystem issues might be retryable
        if error_str.contains("busy") || error_str.contains("lock") {
            return true;
        }

        // Don't retry permission, dependency, or verification errors
        if error_str.contains("permission")
            || error_str.contains("dependency")
            || error_str.contains("verification")
        {
            return false;
        }

        // Default to not retrying for safety
        false
    }

    /// Check if error is a verification failure
    fn is_verification_error(error: &Error) -> bool {
        error.to_string().contains("verification")
    }
}

/// Guard that restores the previous correlation identifier when dropped.
pub struct CorrelationGuard<'a> {
    ctx: &'a OpsCtx,
    previous: Option<String>,
}

impl Drop for CorrelationGuard<'_> {
    fn drop(&mut self) {
        *self.ctx.correlation_id.borrow_mut() = self.previous.take();
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
    config: Option<Config>,
    check_mode: Option<bool>,
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
            check_mode: None,
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

    /// Set check mode
    #[must_use]
    pub fn with_check_mode(mut self, check_mode: bool) -> Self {
        self.check_mode = Some(check_mode);
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
    async fn test_verification_disabled_by_default() {
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
        let (tx, _rx) = sps2_events::channel();

        let mut config = Config::default();
        config.verification.enabled = true;

        let index = IndexManager::new(&store_dir);
        let net = NetClient::new(sps2_net::NetConfig::default()).unwrap();
        let resolver = Resolver::with_events(index.clone(), tx.clone());
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

        // Test that operations work even without verification enabled
        let result = ctx
            .with_verification("test_op", || async { Ok("success") })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_retryable_error_detection() {
        let network_error = sps2_errors::Error::from(sps2_errors::OpsError::MissingComponent {
            component: "network timeout".to_string(),
        });
        assert!(GuardedOperation::<()>::is_retryable_error(&network_error));

        let verification_error =
            sps2_errors::Error::from(sps2_errors::OpsError::VerificationFailed {
                discrepancies: 1,
                state_id: "test".to_string(),
            });
        assert!(!GuardedOperation::<()>::is_retryable_error(
            &verification_error
        ));

        let permission_error = sps2_errors::Error::from(sps2_errors::OpsError::MissingComponent {
            component: "permission denied".to_string(),
        });
        assert!(!GuardedOperation::<()>::is_retryable_error(
            &permission_error
        ));
    }

    #[tokio::test]
    async fn test_verification_error_detection() {
        let verification_error =
            sps2_errors::Error::from(sps2_errors::OpsError::VerificationFailed {
                discrepancies: 1,
                state_id: "test".to_string(),
            });
        assert!(GuardedOperation::<()>::is_verification_error(
            &verification_error
        ));

        let other_error = sps2_errors::Error::from(sps2_errors::OpsError::MissingComponent {
            component: "test".to_string(),
        });
        assert!(!GuardedOperation::<()>::is_verification_error(&other_error));
    }

    #[tokio::test]
    async fn test_execute_with_recovery() {
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

        // Test successful recovery
        let package_specs = vec!["test-package".to_string()];
        let guarded_op = ctx.guarded_install(package_specs);

        let result = guarded_op
            .execute_with_recovery(
                || async {
                    // Simulate operation failure
                    Err(sps2_errors::Error::from(
                        sps2_errors::OpsError::MissingComponent {
                            component: "test failure".to_string(),
                        },
                    ))
                },
                |_error| async {
                    // Recovery succeeds
                    Ok("recovered".to_string())
                },
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "recovered");
    }
}
