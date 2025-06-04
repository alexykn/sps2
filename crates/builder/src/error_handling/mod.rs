// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Advanced error handling and recovery system for package building
//!
//! This module provides comprehensive error handling with recovery strategies,
//! checkpointing, and detailed error reporting for build operations.

use crate::{events::send_event, BuildContext};
use sps2_errors::{BuildError, Error};
use sps2_events::Event;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tokio::fs;
use tokio::time::sleep;

/// Build error handler with recovery strategies
#[derive(Clone)]
pub struct BuildErrorHandler {
    /// Available recovery strategies
    strategies: HashMap<String, Box<dyn RecoveryStrategy>>,
    /// Checkpoint manager for build state
    checkpoint_manager: CheckpointManager,
    /// Maximum retry attempts
    max_retries: usize,
    /// Retry delay configuration
    retry_delay: Duration,
    /// Enable detailed error reporting
    _detailed_errors: bool,
}

/// Recovery strategy trait for different error types
pub trait RecoveryStrategy: Send + Sync {
    /// Clone the strategy
    fn clone_box(&self) -> Box<dyn RecoveryStrategy>;

    /// Check if this strategy can handle the error
    fn can_handle(&self, error: &BuildError) -> bool;

    /// Attempt to recover from the error
    fn recover(&self, error: &BuildError, context: &BuildContext) -> RecoveryAction;

    /// Get recovery description
    fn description(&self) -> &'static str;
}

/// Clone implementation for Box<dyn RecoveryStrategy>
impl Clone for Box<dyn RecoveryStrategy> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Recovery actions that can be taken
#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryAction {
    /// Retry the operation with new configuration
    Retry {
        /// Modified configuration to use
        config_changes: HashMap<String, String>,
        /// Delay before retrying
        delay: Duration,
    },
    /// Skip the failing component
    Skip {
        /// Warning message to emit
        warning: String,
    },
    /// Use an alternative approach
    Alternative {
        /// Alternative method description
        method: String,
        /// Alternative configuration
        config: HashMap<String, String>,
    },
    /// Clean and retry
    CleanRetry {
        /// Paths to clean before retry
        clean_paths: Vec<PathBuf>,
        /// Delay before retrying
        delay: Duration,
    },
    /// Abort with detailed explanation
    Abort {
        /// Detailed error message
        message: String,
        /// Suggested fixes
        suggestions: Vec<String>,
    },
}

/// Build checkpoint for recovery
#[derive(Debug, Clone)]
pub struct BuildCheckpoint {
    /// Checkpoint ID
    pub id: String,
    /// Build stage name
    pub stage: String,
    /// Timestamp of checkpoint
    pub timestamp: SystemTime,
    /// Build state at checkpoint
    pub state: BuildState,
    /// Checkpoint metadata
    pub metadata: HashMap<String, String>,
}

/// Build state captured at checkpoint
#[derive(Debug, Clone)]
pub struct BuildState {
    /// Environment variables
    pub env_vars: HashMap<String, String>,
    /// Completed steps
    pub completed_steps: Vec<String>,
    /// Generated artifacts
    pub artifacts: Vec<PathBuf>,
    /// Build configuration
    pub config: HashMap<String, String>,
}

/// Checkpoint manager for build recovery
#[derive(Clone)]
pub struct CheckpointManager {
    /// Checkpoint storage directory
    checkpoint_dir: PathBuf,
    /// Active checkpoints
    checkpoints: Vec<BuildCheckpoint>,
    /// Maximum checkpoints to keep
    max_checkpoints: usize,
}

// Recovery strategy implementations

/// Dependency conflict recovery strategy
#[derive(Clone)]
pub struct DependencyConflictRecovery;

impl RecoveryStrategy for DependencyConflictRecovery {
    fn clone_box(&self) -> Box<dyn RecoveryStrategy> {
        Box::new(self.clone())
    }

    fn can_handle(&self, error: &BuildError) -> bool {
        matches!(error, BuildError::DependencyConflict { .. })
    }

    fn recover(&self, error: &BuildError, _context: &BuildContext) -> RecoveryAction {
        if let BuildError::DependencyConflict { message } = error {
            // Extract dependency information from error message
            if message.contains("version") {
                RecoveryAction::Alternative {
                    method: "Use compatible dependency version".to_string(),
                    config: HashMap::from([
                        ("dependency_resolution".to_string(), "flexible".to_string()),
                        ("allow_older_versions".to_string(), "true".to_string()),
                    ]),
                }
            } else {
                RecoveryAction::Abort {
                    message: format!("Cannot resolve dependency conflict: {message}"),
                    suggestions: vec![
                        "Check dependency version constraints".to_string(),
                        "Update recipe with compatible versions".to_string(),
                        "Consider using vendored dependencies".to_string(),
                    ],
                }
            }
        } else {
            RecoveryAction::Abort {
                message: "Unexpected error type".to_string(),
                suggestions: vec![],
            }
        }
    }

    fn description(&self) -> &'static str {
        "Dependency conflict recovery"
    }
}

/// Compilation failure recovery strategy
#[derive(Clone)]
pub struct CompilationFailedRecovery;

impl RecoveryStrategy for CompilationFailedRecovery {
    fn clone_box(&self) -> Box<dyn RecoveryStrategy> {
        Box::new(self.clone())
    }

    fn can_handle(&self, error: &BuildError) -> bool {
        matches!(
            error,
            BuildError::CompilationFailed { .. } | BuildError::CompileFailed { .. }
        )
    }

    fn recover(&self, error: &BuildError, _context: &BuildContext) -> RecoveryAction {
        let message = match error {
            BuildError::CompilationFailed { message } | BuildError::CompileFailed { message } => {
                message
            }
            _ => {
                return RecoveryAction::Abort {
                    message: "Unexpected error type".to_string(),
                    suggestions: vec![],
                }
            }
        };

        // Analyze compilation error
        if message.contains("out of memory") || message.contains("OOM") {
            RecoveryAction::Retry {
                config_changes: HashMap::from([
                    ("parallel_jobs".to_string(), "1".to_string()),
                    ("optimization_level".to_string(), "0".to_string()),
                ]),
                delay: Duration::from_secs(5),
            }
        } else if message.contains("undefined reference") || message.contains("unresolved symbol") {
            RecoveryAction::Alternative {
                method: "Add missing libraries".to_string(),
                config: HashMap::from([
                    ("link_flags".to_string(), "-lm -ldl -lpthread".to_string()),
                    (
                        "pkg_config_path".to_string(),
                        "/opt/pm/lib/pkgconfig".to_string(),
                    ),
                ]),
            }
        } else if message.contains("compiler") || message.contains("cc1") {
            RecoveryAction::Alternative {
                method: "Try different compiler flags".to_string(),
                config: HashMap::from([
                    ("cflags".to_string(), "-O2 -fPIC".to_string()),
                    ("cxxflags".to_string(), "-O2 -fPIC -std=c++17".to_string()),
                ]),
            }
        } else {
            RecoveryAction::Abort {
                message: format!("Compilation failed: {message}"),
                suggestions: vec![
                    "Check build dependencies are installed".to_string(),
                    "Review compiler output for specific errors".to_string(),
                    "Try building with verbose output enabled".to_string(),
                    "Check if patches are needed for this platform".to_string(),
                ],
            }
        }
    }

    fn description(&self) -> &'static str {
        "Compilation failure recovery"
    }
}

/// Test failure recovery strategy
#[derive(Clone)]
pub struct TestsFailedRecovery;

impl RecoveryStrategy for TestsFailedRecovery {
    fn clone_box(&self) -> Box<dyn RecoveryStrategy> {
        Box::new(self.clone())
    }

    fn can_handle(&self, error: &BuildError) -> bool {
        matches!(error, BuildError::TestsFailed { .. })
    }

    fn recover(&self, error: &BuildError, context: &BuildContext) -> RecoveryAction {
        if let BuildError::TestsFailed { passed, total } = error {
            let failure_rate = 1.0 - (*passed as f64 / *total as f64);

            if failure_rate < 0.1 {
                // Less than 10% failures - allow skipping
                RecoveryAction::Skip {
                    warning: format!(
                        "Skipping test failures ({}/{} passed) for {}",
                        passed, total, context.name
                    ),
                }
            } else if failure_rate < 0.5 {
                // Less than 50% failures - retry with different settings
                RecoveryAction::Retry {
                    config_changes: HashMap::from([
                        ("test_timeout".to_string(), "300".to_string()),
                        ("test_parallel".to_string(), "false".to_string()),
                    ]),
                    delay: Duration::from_secs(2),
                }
            } else {
                // High failure rate - abort
                RecoveryAction::Abort {
                    message: format!("Too many test failures: {}/{} passed", passed, total),
                    suggestions: vec![
                        "Review test output for specific failures".to_string(),
                        "Check if tests require specific environment setup".to_string(),
                        "Consider disabling flaky tests in recipe".to_string(),
                    ],
                }
            }
        } else {
            RecoveryAction::Abort {
                message: "Unexpected error type".to_string(),
                suggestions: vec![],
            }
        }
    }

    fn description(&self) -> &'static str {
        "Test failure recovery"
    }
}

/// Network error recovery strategy
#[derive(Clone)]
pub struct NetworkErrorRecovery {
    /// Maximum network retries
    max_retries: usize,
}

impl NetworkErrorRecovery {
    /// Create new network error recovery
    #[must_use]
    pub fn new() -> Self {
        Self { max_retries: 3 }
    }
}

impl Default for NetworkErrorRecovery {
    fn default() -> Self {
        Self::new()
    }
}

impl RecoveryStrategy for NetworkErrorRecovery {
    fn clone_box(&self) -> Box<dyn RecoveryStrategy> {
        Box::new(self.clone())
    }

    fn can_handle(&self, error: &BuildError) -> bool {
        matches!(
            error,
            BuildError::FetchFailed { .. }
                | BuildError::NetworkAccessDenied
                | BuildError::NetworkDisabled { .. }
        )
    }

    fn recover(&self, error: &BuildError, _context: &BuildContext) -> RecoveryAction {
        match error {
            BuildError::FetchFailed { url: _ } => RecoveryAction::Retry {
                config_changes: HashMap::from([
                    ("fetch_timeout".to_string(), "600".to_string()),
                    ("fetch_retries".to_string(), self.max_retries.to_string()),
                    ("use_mirrors".to_string(), "true".to_string()),
                ]),
                delay: Duration::from_secs(10),
            },
            BuildError::NetworkAccessDenied | BuildError::NetworkDisabled { .. } => {
                RecoveryAction::Alternative {
                    method: "Use local cache or offline mode".to_string(),
                    config: HashMap::from([
                        ("offline_mode".to_string(), "true".to_string()),
                        ("use_local_cache".to_string(), "true".to_string()),
                    ]),
                }
            }
            _ => RecoveryAction::Abort {
                message: "Unexpected error type".to_string(),
                suggestions: vec![],
            },
        }
    }

    fn description(&self) -> &'static str {
        "Network error recovery"
    }
}

/// Disk space recovery strategy
#[derive(Clone)]
pub struct DiskSpaceRecovery;

impl RecoveryStrategy for DiskSpaceRecovery {
    fn clone_box(&self) -> Box<dyn RecoveryStrategy> {
        Box::new(self.clone())
    }

    fn can_handle(&self, error: &BuildError) -> bool {
        if let BuildError::Failed { message } = error {
            message.contains("space") || message.contains("disk full") || message.contains("ENOSPC")
        } else {
            false
        }
    }

    fn recover(&self, error: &BuildError, context: &BuildContext) -> RecoveryAction {
        if let BuildError::Failed { .. } = error {
            RecoveryAction::CleanRetry {
                clean_paths: vec![
                    context.output_dir.join("build"),
                    context.output_dir.join("tmp"),
                    PathBuf::from("/tmp").join(format!("sps2-build-{}", context.name)),
                ],
                delay: Duration::from_secs(5),
            }
        } else {
            RecoveryAction::Abort {
                message: "Unexpected error type".to_string(),
                suggestions: vec![],
            }
        }
    }

    fn description(&self) -> &'static str {
        "Disk space recovery"
    }
}

impl BuildErrorHandler {
    /// Create new build error handler
    #[must_use]
    pub fn new(checkpoint_dir: PathBuf) -> Self {
        let mut strategies: HashMap<String, Box<dyn RecoveryStrategy>> = HashMap::new();

        // Register default strategies
        strategies.insert(
            "dependency_conflict".to_string(),
            Box::new(DependencyConflictRecovery),
        );
        strategies.insert(
            "compilation_failed".to_string(),
            Box::new(CompilationFailedRecovery),
        );
        strategies.insert("tests_failed".to_string(), Box::new(TestsFailedRecovery));
        strategies.insert(
            "network_error".to_string(),
            Box::new(NetworkErrorRecovery::new()),
        );
        strategies.insert("disk_space".to_string(), Box::new(DiskSpaceRecovery));

        Self {
            strategies,
            checkpoint_manager: CheckpointManager::new(checkpoint_dir),
            max_retries: 3,
            retry_delay: Duration::from_secs(5),
            _detailed_errors: true,
        }
    }

    /// Set maximum retry attempts
    #[must_use]
    pub fn with_max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set retry delay
    #[must_use]
    pub fn with_retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    /// Register custom recovery strategy
    pub fn register_strategy(&mut self, name: String, strategy: Box<dyn RecoveryStrategy>) {
        self.strategies.insert(name, strategy);
    }

    /// Handle build error with recovery attempts
    pub async fn handle_error(
        &mut self,
        error: BuildError,
        context: &BuildContext,
        attempt: usize,
    ) -> Result<RecoveryAction, Error> {
        // Send error event
        send_event(
            context,
            Event::BuildFailed {
                package: context.name.clone(),
                version: context.version.clone(),
                error: error.to_string(),
            },
        );

        // Check if we've exceeded max retries
        if attempt >= self.max_retries {
            return Ok(RecoveryAction::Abort {
                message: format!("Max retries ({}) exceeded", self.max_retries),
                suggestions: vec!["Manual intervention required".to_string()],
            });
        }

        // Find appropriate recovery strategy
        for strategy in self.strategies.values() {
            if strategy.can_handle(&error) {
                let action = strategy.recover(&error, context);

                // Log recovery attempt
                send_event(
                    context,
                    Event::BuildRetrying {
                        package: context.name.clone(),
                        attempt,
                        reason: format!("{}: {}", strategy.description(), error),
                    },
                );

                return Ok(action);
            }
        }

        // No strategy found - create detailed error
        Ok(self.create_detailed_abort(&error))
    }

    /// Create detailed abort action with suggestions
    fn create_detailed_abort(&self, error: &BuildError) -> RecoveryAction {
        let suggestions = match error {
            BuildError::MissingBuildDep { name } => vec![
                format!("Install missing dependency: {name}"),
                "Update recipe build dependencies".to_string(),
                "Check if dependency name has changed".to_string(),
            ],
            BuildError::PatchFailed { patch } => vec![
                format!("Review patch file: {patch}"),
                "Check if patch is applicable to this version".to_string(),
                "Try applying patch manually to debug".to_string(),
            ],
            BuildError::ConfigureFailed { message } => vec![
                "Check configure log for details".to_string(),
                format!("Common issue: {message}"),
                "Ensure all required tools are in PATH".to_string(),
            ],
            BuildError::Timeout { seconds } => vec![
                format!("Build exceeded {seconds}s timeout"),
                "Consider increasing timeout in recipe".to_string(),
                "Check for infinite loops or hanging processes".to_string(),
            ],
            _ => vec!["Check build logs for more details".to_string()],
        };

        RecoveryAction::Abort {
            message: format!("Build failed: {error}"),
            suggestions,
        }
    }

    /// Create checkpoint for current build state
    pub async fn create_checkpoint(
        &mut self,
        stage_name: String,
        state: BuildState,
        metadata: HashMap<String, String>,
    ) -> Result<String, Error> {
        self.checkpoint_manager
            .create_checkpoint(stage_name, state, metadata)
            .await
    }

    /// Restore from checkpoint
    pub async fn restore_checkpoint(&self, checkpoint_id: &str) -> Result<BuildCheckpoint, Error> {
        self.checkpoint_manager
            .restore_checkpoint(checkpoint_id)
            .await
    }

    /// Clean old checkpoints
    pub async fn clean_checkpoints(&mut self) -> Result<(), Error> {
        self.checkpoint_manager.clean_old_checkpoints().await
    }
}

impl CheckpointManager {
    /// Create new checkpoint manager
    #[must_use]
    pub fn new(checkpoint_dir: PathBuf) -> Self {
        Self {
            checkpoint_dir,
            checkpoints: Vec::new(),
            max_checkpoints: 10,
        }
    }

    /// Create a new checkpoint
    pub async fn create_checkpoint(
        &mut self,
        stage_name: String,
        state: BuildState,
        metadata: HashMap<String, String>,
    ) -> Result<String, Error> {
        let checkpoint_id = format!("{}-{}", stage_name, chrono::Utc::now().timestamp());
        let checkpoint = BuildCheckpoint {
            id: checkpoint_id.clone(),
            stage: stage_name,
            timestamp: SystemTime::now(),
            state,
            metadata,
        };

        // Ensure checkpoint directory exists
        fs::create_dir_all(&self.checkpoint_dir)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to create checkpoint directory: {e}"),
            })?;

        // Save checkpoint to disk
        let checkpoint_path = self.checkpoint_dir.join(format!("{checkpoint_id}.json"));
        let checkpoint_data =
            serde_json::to_string_pretty(&checkpoint).map_err(|e| BuildError::Failed {
                message: format!("Failed to serialize checkpoint: {e}"),
            })?;

        fs::write(&checkpoint_path, checkpoint_data)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to write checkpoint: {e}"),
            })?;

        self.checkpoints.push(checkpoint.clone());

        // Clean old checkpoints if needed
        if self.checkpoints.len() > self.max_checkpoints {
            self.clean_old_checkpoints().await?;
        }

        Ok(checkpoint_id)
    }

    /// Restore from checkpoint
    pub async fn restore_checkpoint(&self, checkpoint_id: &str) -> Result<BuildCheckpoint, Error> {
        let checkpoint_path = self.checkpoint_dir.join(format!("{checkpoint_id}.json"));

        let checkpoint_data =
            fs::read_to_string(&checkpoint_path)
                .await
                .map_err(|e| BuildError::Failed {
                    message: format!("Failed to read checkpoint: {e}"),
                })?;

        serde_json::from_str(&checkpoint_data).map_err(|e| {
            BuildError::Failed {
                message: format!("Failed to deserialize checkpoint: {e}"),
            }
            .into()
        })
    }

    /// Clean old checkpoints
    pub async fn clean_old_checkpoints(&mut self) -> Result<(), Error> {
        // Sort by timestamp
        self.checkpoints.sort_by_key(|c| c.timestamp);

        // Remove oldest checkpoints
        while self.checkpoints.len() > self.max_checkpoints {
            if let Some(old_checkpoint) = self.checkpoints.first() {
                let checkpoint_path = self
                    .checkpoint_dir
                    .join(format!("{}.json", old_checkpoint.id));
                let _ = fs::remove_file(&checkpoint_path).await;
                self.checkpoints.remove(0);
            }
        }

        Ok(())
    }

    /// List all checkpoints
    #[must_use]
    pub fn list_checkpoints(&self) -> &[BuildCheckpoint] {
        &self.checkpoints
    }

    /// Get latest checkpoint for stage
    #[must_use]
    pub fn get_latest_checkpoint(&self, stage: &str) -> Option<&BuildCheckpoint> {
        self.checkpoints
            .iter()
            .filter(|c| c.stage == stage)
            .max_by_key(|c| c.timestamp)
    }
}

/// Execute build operation with error recovery
pub async fn with_recovery<F, T>(
    handler: &mut BuildErrorHandler,
    context: &BuildContext,
    operation: F,
) -> Result<T, Error>
where
    F: Fn() -> futures::future::BoxFuture<'static, Result<T, BuildError>> + Clone,
    T: Send + 'static,
{
    let mut attempt = 0;
    let mut _last_error = None;

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                _last_error = Some(error.clone());

                match handler.handle_error(error, context, attempt).await? {
                    RecoveryAction::Retry { delay, .. } => {
                        sleep(delay).await;
                        attempt += 1;
                    }
                    RecoveryAction::Skip { warning } => {
                        send_event(
                            context,
                            Event::BuildWarning {
                                package: context.name.clone(),
                                message: warning,
                            },
                        );
                        return Err(BuildError::Failed {
                            message: "Operation skipped due to error".to_string(),
                        }
                        .into());
                    }
                    RecoveryAction::Alternative { method, .. } => {
                        send_event(
                            context,
                            Event::BuildWarning {
                                package: context.name.clone(),
                                message: format!("Using alternative method: {method}"),
                            },
                        );
                        // Alternative would be implemented by caller
                        return Err(BuildError::Failed {
                            message: format!("Alternative method required: {method}"),
                        }
                        .into());
                    }
                    RecoveryAction::CleanRetry { clean_paths, delay } => {
                        for path in clean_paths {
                            let _ = fs::remove_dir_all(&path).await;
                        }
                        sleep(delay).await;
                        attempt += 1;
                    }
                    RecoveryAction::Abort {
                        message,
                        suggestions,
                    } => {
                        send_event(
                            context,
                            Event::BuildFailed {
                                package: context.name.clone(),
                                version: context.version.clone(),
                                error: message.clone(),
                            },
                        );

                        // Log suggestions
                        for suggestion in suggestions {
                            send_event(
                                context,
                                Event::BuildWarning {
                                    package: context.name.clone(),
                                    message: format!("Suggestion: {suggestion}"),
                                },
                            );
                        }

                        return Err(BuildError::Failed { message }.into());
                    }
                }
            }
        }
    }
}

// Serde implementations for serialization
use serde::{Deserialize, Serialize};

impl Serialize for BuildCheckpoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct CheckpointData {
            id: String,
            stage: String,
            timestamp: u64,
            state: BuildState,
            metadata: HashMap<String, String>,
        }

        let data = CheckpointData {
            id: self.id.clone(),
            stage: self.stage.clone(),
            timestamp: self
                .timestamp
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            state: self.state.clone(),
            metadata: self.metadata.clone(),
        };

        data.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BuildCheckpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct CheckpointData {
            id: String,
            stage: String,
            timestamp: u64,
            state: BuildState,
            metadata: HashMap<String, String>,
        }

        let data = CheckpointData::deserialize(deserializer)?;

        Ok(BuildCheckpoint {
            id: data.id,
            stage: data.stage,
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(data.timestamp),
            state: data.state,
            metadata: data.metadata,
        })
    }
}

impl Serialize for BuildState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct StateData {
            env_vars: HashMap<String, String>,
            completed_steps: Vec<String>,
            artifacts: Vec<String>,
            config: HashMap<String, String>,
        }

        let data = StateData {
            env_vars: self.env_vars.clone(),
            completed_steps: self.completed_steps.clone(),
            artifacts: self
                .artifacts
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            config: self.config.clone(),
        };

        data.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BuildState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct StateData {
            env_vars: HashMap<String, String>,
            completed_steps: Vec<String>,
            artifacts: Vec<String>,
            config: HashMap<String, String>,
        }

        let data = StateData::deserialize(deserializer)?;

        Ok(BuildState {
            env_vars: data.env_vars,
            completed_steps: data.completed_steps,
            artifacts: data.artifacts.into_iter().map(PathBuf::from).collect(),
            config: data.config,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_types::Version;
    use tempfile::tempdir;

    #[test]
    fn test_recovery_strategies() {
        let dep_recovery = DependencyConflictRecovery;
        assert!(dep_recovery.can_handle(&BuildError::DependencyConflict {
            message: "test".to_string()
        }));

        let compile_recovery = CompilationFailedRecovery;
        assert!(compile_recovery.can_handle(&BuildError::CompilationFailed {
            message: "test".to_string()
        }));

        let test_recovery = TestsFailedRecovery;
        assert!(test_recovery.can_handle(&BuildError::TestsFailed {
            passed: 5,
            total: 10
        }));

        let network_recovery = NetworkErrorRecovery::new();
        assert!(network_recovery.can_handle(&BuildError::FetchFailed {
            url: "http://test".to_string()
        }));

        let disk_recovery = DiskSpaceRecovery;
        assert!(disk_recovery.can_handle(&BuildError::Failed {
            message: "disk full".to_string()
        }));
    }

    #[tokio::test]
    async fn test_checkpoint_manager() {
        let temp = tempdir().unwrap();
        let mut manager = CheckpointManager::new(temp.path().to_path_buf());

        let state = BuildState {
            env_vars: HashMap::from([("TEST".to_string(), "value".to_string())]),
            completed_steps: vec!["configure".to_string()],
            artifacts: vec![PathBuf::from("/tmp/test.o")],
            config: HashMap::new(),
        };

        let checkpoint_id = manager
            .create_checkpoint("build".to_string(), state, HashMap::new())
            .await
            .unwrap();

        assert!(!checkpoint_id.is_empty());
        assert_eq!(manager.checkpoints.len(), 1);

        let restored = manager.restore_checkpoint(&checkpoint_id).await.unwrap();
        assert_eq!(restored.stage, "build");
        assert_eq!(restored.state.completed_steps, vec!["configure"]);
    }

    #[tokio::test]
    async fn test_error_handler() {
        let temp = tempdir().unwrap();
        let mut handler = BuildErrorHandler::new(temp.path().to_path_buf());

        let context = BuildContext::new(
            "test".to_string(),
            Version::parse("1.0.0").unwrap(),
            PathBuf::from("test.star"),
            temp.path().to_path_buf(),
        );

        let error = BuildError::TestsFailed {
            passed: 9,
            total: 10,
        };

        let action = handler.handle_error(error, &context, 0).await.unwrap();
        match action {
            RecoveryAction::Skip { warning } => {
                assert!(warning.contains("Skipping test failures"));
            }
            _ => panic!("Expected skip action"),
        }
    }

    #[test]
    fn test_recovery_actions() {
        let retry = RecoveryAction::Retry {
            config_changes: HashMap::new(),
            delay: Duration::from_secs(1),
        };
        assert_eq!(
            retry,
            RecoveryAction::Retry {
                config_changes: HashMap::new(),
                delay: Duration::from_secs(1),
            }
        );

        let skip = RecoveryAction::Skip {
            warning: "test".to_string(),
        };
        assert_eq!(
            skip,
            RecoveryAction::Skip {
                warning: "test".to_string(),
            }
        );
    }
}
