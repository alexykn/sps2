//! Core platform abstractions and context management

use sps2_errors::PlatformError;
use sps2_events::AppEvent;
use std::collections::HashMap;
use std::future::Future;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::binary::BinaryOperations;
use crate::filesystem::FilesystemOperations;
use crate::process::ProcessOperations;

/// Context for platform operations, providing event emission and metadata tracking
pub struct PlatformContext {
    event_sender: Option<mpsc::UnboundedSender<AppEvent>>,
    operation_metadata: HashMap<String, String>,
}

impl PlatformContext {
    /// Create a new platform context with event emission capabilities
    pub fn new(event_sender: Option<mpsc::UnboundedSender<AppEvent>>) -> Self {
        Self {
            event_sender,
            operation_metadata: HashMap::new(),
        }
    }

    /// Emit a platform event if event sender is available
    pub async fn emit_event(&self, event: AppEvent) {
        if let Some(sender) = &self.event_sender {
            let _ = sender.send(event);
        }
    }

    /// Execute an operation with automatic event emission
    pub async fn execute_with_events<T, F>(
        &self,
        _operation: &str,
        f: F,
    ) -> Result<T, PlatformError>
    where
        F: Future<Output = Result<T, PlatformError>>,
    {
        let start = Instant::now();

        // TODO: Emit operation started event

        let result = f.await;
        let _duration = start.elapsed();

        // TODO: Emit operation completed/failed event based on result

        result
    }
}

/// Main platform abstraction providing access to all platform operations
pub struct Platform {
    binary_ops: Box<dyn BinaryOperations>,
    filesystem_ops: Box<dyn FilesystemOperations>,
    process_ops: Box<dyn ProcessOperations>,
}

impl Platform {
    /// Create a new platform instance with the specified implementations
    pub fn new(
        binary_ops: Box<dyn BinaryOperations>,
        filesystem_ops: Box<dyn FilesystemOperations>,
        process_ops: Box<dyn ProcessOperations>,
    ) -> Self {
        Self {
            binary_ops,
            filesystem_ops,
            process_ops,
        }
    }

    /// Get the current platform (macOS in our case)
    pub fn current() -> Self {
        use crate::implementations::macos::{
            binary::MacOSBinaryOperations, filesystem::MacOSFilesystemOperations,
            process::MacOSProcessOperations,
        };

        Self::new(
            Box::new(MacOSBinaryOperations::new()),
            Box::new(MacOSFilesystemOperations::new()),
            Box::new(MacOSProcessOperations::new()),
        )
    }

    /// Access binary operations
    pub fn binary(&self) -> &dyn BinaryOperations {
        &*self.binary_ops
    }

    /// Access filesystem operations
    pub fn filesystem(&self) -> &dyn FilesystemOperations {
        &*self.filesystem_ops
    }

    /// Access process operations
    pub fn process(&self) -> &dyn ProcessOperations {
        &*self.process_ops
    }

    /// Create a platform context with event emission
    pub fn create_context(
        &self,
        event_sender: Option<mpsc::UnboundedSender<AppEvent>>,
    ) -> PlatformContext {
        PlatformContext::new(event_sender)
    }

    /// Convenience method: Clone a file using APFS clonefile
    pub async fn clone_file(
        &self,
        ctx: &PlatformContext,
        src: &std::path::Path,
        dst: &std::path::Path,
    ) -> Result<(), sps2_errors::PlatformError> {
        self.filesystem().clone_file(ctx, src, dst).await
    }

    /// Convenience method: Get binary dependencies
    pub async fn get_dependencies(
        &self,
        ctx: &PlatformContext,
        binary: &std::path::Path,
    ) -> Result<Vec<String>, sps2_errors::PlatformError> {
        self.binary().get_dependencies(ctx, binary).await
    }

    /// Convenience method: Execute a command and get output
    pub async fn execute_command(
        &self,
        ctx: &PlatformContext,
        cmd: crate::process::PlatformCommand,
    ) -> Result<crate::process::CommandOutput, sps2_errors::Error> {
        self.process().execute_command(ctx, cmd).await
    }

    /// Convenience method: Create a new command builder
    pub fn command(&self, program: &str) -> crate::process::PlatformCommand {
        self.process().create_command(program)
    }

    /// Convenience method: Sign a binary
    pub async fn sign_binary(
        &self,
        ctx: &PlatformContext,
        binary: &std::path::Path,
        identity: Option<&str>,
    ) -> Result<(), sps2_errors::PlatformError> {
        self.binary().sign_binary(ctx, binary, identity).await
    }

    /// Convenience method: Atomically rename a file
    pub async fn atomic_rename(
        &self,
        ctx: &PlatformContext,
        src: &std::path::Path,
        dst: &std::path::Path,
    ) -> Result<(), sps2_errors::PlatformError> {
        self.filesystem().atomic_rename(ctx, src, dst).await
    }
}

/// Integration helpers for converting from other context types
impl PlatformContext {
    /// Create a PlatformContext with basic package information (fallback when BuildContext is not available)
    pub fn with_package_info(
        event_sender: Option<mpsc::UnboundedSender<AppEvent>>,
        package_name: &str,
        package_version: &str,
        arch: &str,
    ) -> Self {
        let mut metadata = HashMap::new();
        metadata.insert("package_name".to_string(), package_name.to_string());
        metadata.insert("package_version".to_string(), package_version.to_string());
        metadata.insert("package_arch".to_string(), arch.to_string());

        Self {
            event_sender,
            operation_metadata: metadata,
        }
    }

    /// Get package metadata if available
    pub fn get_package_name(&self) -> Option<&String> {
        self.operation_metadata.get("package_name")
    }

    /// Get package version if available  
    pub fn get_package_version(&self) -> Option<&String> {
        self.operation_metadata.get("package_version")
    }

    /// Get package architecture if available
    pub fn get_package_arch(&self) -> Option<&String> {
        self.operation_metadata.get("package_arch")
    }

    /// Add custom metadata to the context
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.operation_metadata.insert(key, value);
    }

    /// Get all metadata
    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.operation_metadata
    }
}
