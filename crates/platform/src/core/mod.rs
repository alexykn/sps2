//! Core platform abstractions and context management

use crate::binary::BinaryOperations;
use crate::filesystem::FilesystemOperations;
use crate::process::ProcessOperations;
use serde::{Deserialize, Serialize};
use sps2_errors::PlatformError;
use sps2_events::{
    events::{
        FailureContext, PlatformEvent, PlatformOperationContext, PlatformOperationKind,
        PlatformOperationMetrics,
    },
    AppEvent, EventEmitter, EventSender,
};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

/// Context for platform operations, providing event emission and metadata tracking
pub struct PlatformContext {
    event_sender: Option<EventSender>,
    operation_metadata: HashMap<String, String>,
}

impl PlatformContext {
    /// Create a new platform context with event emission capabilities
    pub fn new(event_sender: Option<EventSender>) -> Self {
        Self {
            event_sender,
            operation_metadata: HashMap::new(),
        }
    }

    /// Emit a platform event if event sender is available
    pub async fn emit_event(&self, event: AppEvent) {
        if let Some(sender) = &self.event_sender {
            sender.emit(event);
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

    /// Get access to the platform manager for tool registry and other services
    pub fn platform_manager(&self) -> &'static PlatformManager {
        PlatformManager::instance()
    }
}

/// Platform capabilities detected at runtime
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    pub install_name_tool_version: Option<String>,
    pub otool_available: bool,
    pub codesign_available: bool,
    pub apfs_clonefile_supported: bool,
    pub atomic_operations_supported: bool,
}

impl Default for PlatformCapabilities {
    fn default() -> Self {
        Self {
            install_name_tool_version: None,
            otool_available: true,             // Assume available on macOS
            codesign_available: true,          // Assume available on macOS
            apfs_clonefile_supported: true,    // Assume APFS on macOS
            atomic_operations_supported: true, // Assume supported on macOS
        }
    }
}

/// Tool information with caching
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub path: std::path::PathBuf,
    pub version: String,
    pub last_verified: Instant,
}

/// Registry for caching tool information with comprehensive platform tool support
#[derive(Debug)]
pub struct ToolRegistry {
    /// Cached tool information with thread-safe access
    tools: Arc<RwLock<HashMap<String, CachedTool>>>,

    /// Search paths for tool discovery (in priority order)
    search_paths: Vec<PathBuf>,

    /// Tool-specific fallback paths for common tools
    fallback_paths: HashMap<String, Vec<PathBuf>>,

    /// Event sender for tool discovery notifications (with interior mutability)
    event_tx: Arc<RwLock<Option<EventSender>>>,
}

fn duration_to_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Persistent platform cache for storing discovered tools across process restarts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCache {
    /// Platform type (e.g., "macos")
    pub platform_type: String,

    /// When the cache was last updated
    pub discovered_at: String,

    /// Discovered tool paths (user-configurable)
    pub tools: HashMap<String, String>,

    /// Platform capabilities detected
    pub capabilities: PlatformCapabilities,
}

impl PlatformCache {
    /// Get the default cache file path: ~/.config/sps2/.platform.json
    pub fn default_path() -> Result<PathBuf, PlatformError> {
        let home_dir = dirs::home_dir().ok_or_else(|| PlatformError::ConfigError {
            message: "home directory not found".to_string(),
        })?;
        Ok(home_dir.join(".config").join("sps2").join(".platform.json"))
    }

    /// Load platform cache from file
    pub async fn load() -> Result<Option<Self>, PlatformError> {
        let cache_path = Self::default_path()?;

        if !cache_path.exists() {
            return Ok(None);
        }

        let contents = tokio::fs::read_to_string(&cache_path).await.map_err(|e| {
            PlatformError::ConfigError {
                message: format!("failed to read platform cache: {e}"),
            }
        })?;

        let cache: Self =
            serde_json::from_str(&contents).map_err(|e| PlatformError::ConfigError {
                message: format!("failed to parse platform cache: {e}"),
            })?;

        Ok(Some(cache))
    }

    /// Save platform cache to file
    pub async fn save(&self) -> Result<(), PlatformError> {
        let cache_path = Self::default_path()?;

        // Ensure parent directory exists
        if let Some(parent) = cache_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| PlatformError::ConfigError {
                    message: format!("failed to create config directory: {e}"),
                })?;
        }

        let contents =
            serde_json::to_string_pretty(self).map_err(|e| PlatformError::ConfigError {
                message: format!("failed to serialize platform cache: {e}"),
            })?;

        tokio::fs::write(&cache_path, contents)
            .await
            .map_err(|e| PlatformError::ConfigError {
                message: format!("failed to write platform cache: {e}"),
            })?;

        Ok(())
    }

    /// Create new cache from current platform state
    pub fn new(platform_type: String, capabilities: PlatformCapabilities) -> Self {
        Self {
            platform_type,
            discovered_at: chrono::Utc::now().to_rfc3339(),
            tools: HashMap::new(),
            capabilities,
        }
    }
}

/// Cached information about a discovered tool
#[derive(Debug, Clone)]
pub struct CachedTool {
    /// Full path to the tool executable
    pub path: PathBuf,

    /// When this tool was last verified to exist
    pub verified_at: SystemTime,

    /// Tool version if detectable
    pub version: Option<String>,

    /// Tool capabilities and metadata
    pub metadata: ToolMetadata,
}

/// Metadata about tool capabilities and requirements
#[derive(Debug, Clone)]
pub struct ToolMetadata {
    /// Tool category for organization
    pub category: ToolCategory,

    /// Whether this tool is critical for platform operations
    pub is_critical: bool,

    /// Installation suggestion for this tool
    pub install_suggestion: String,
}

/// Categories of tools for organization and priority
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCategory {
    /// Platform-critical tools (otool, install_name_tool, codesign)
    PlatformCritical,

    /// Build system tools (make, cmake, gcc, clang)
    BuildSystem,

    /// Development utilities (pkg-config, autotools)
    Development,

    /// System utilities (which)
    System,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
impl ToolRegistry {
    /// Create a new tool registry with comprehensive tool support and persistent caching
    pub fn new() -> Self {
        let mut fallback_paths = HashMap::new();

        // Platform-critical tools (Xcode Command Line Tools)
        let xcode_paths = vec![
            PathBuf::from("/usr/bin"),
            PathBuf::from("/Applications/Xcode.app/Contents/Developer/usr/bin"),
            PathBuf::from("/Library/Developer/CommandLineTools/usr/bin"),
        ];

        fallback_paths.insert("otool".to_string(), xcode_paths.clone());
        fallback_paths.insert("install_name_tool".to_string(), xcode_paths.clone());
        fallback_paths.insert("codesign".to_string(), xcode_paths.clone());

        // Build system tools (common locations)
        let build_paths = vec![
            PathBuf::from("/usr/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/opt/local/bin"), // MacPorts
        ];

        for tool in &[
            "make",
            "cmake",
            "gcc",
            "clang",
            "configure",
            "autoconf",
            "automake",
            "libtool",
            "pkg-config",
            "ninja",
            "meson",
        ] {
            fallback_paths.insert(tool.to_string(), build_paths.clone());
        }

        // System tools
        fallback_paths.insert("which".to_string(), vec![PathBuf::from("/usr/bin")]);

        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),

            search_paths: vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/Applications/Xcode.app/Contents/Developer/usr/bin"),
                PathBuf::from("/Library/Developer/CommandLineTools/usr/bin"),
            ],
            fallback_paths,
            event_tx: Arc::new(RwLock::new(None)),
        }
    }

    fn tool_operation_context(
        operation: &str,
        tool: &str,
        path: Option<&Path>,
    ) -> PlatformOperationContext {
        PlatformOperationContext {
            kind: PlatformOperationKind::ToolDiscovery,
            operation: format!("{operation}:{tool}"),
            target: path.map(Path::to_path_buf),
            source: None,
            command: None,
        }
    }

    fn tool_operation_metrics(
        duration: Option<Duration>,
        search_paths: &[PathBuf],
        notes: Vec<String>,
    ) -> Option<PlatformOperationMetrics> {
        let duration_ms = duration.map(duration_to_millis);
        let mut changes = notes;
        changes.extend(
            search_paths
                .iter()
                .map(|path| format!("search_path={}", path.display())),
        );

        if duration_ms.is_none() && changes.is_empty() {
            return None;
        }

        Some(PlatformOperationMetrics {
            duration_ms,
            exit_code: None,
            stdout_bytes: None,
            stderr_bytes: None,
            changes: if changes.is_empty() {
                None
            } else {
                Some(changes)
            },
        })
    }

    fn emit_tool_operation_started(&self, operation: &str, tool: &str, path: Option<&Path>) {
        let context = Self::tool_operation_context(operation, tool, path);
        self.emit_event(AppEvent::Platform(PlatformEvent::OperationStarted {
            context,
        }));
    }

    fn emit_tool_operation_completed(
        &self,
        operation: &str,
        tool: &str,
        path: Option<&Path>,
        duration: Option<Duration>,
        search_paths: &[PathBuf],
        notes: Vec<String>,
    ) {
        let context = Self::tool_operation_context(operation, tool, path);
        let metrics = Self::tool_operation_metrics(duration, search_paths, notes);
        self.emit_event(AppEvent::Platform(PlatformEvent::OperationCompleted {
            context,
            metrics,
        }));
    }

    fn emit_tool_operation_failed(
        &self,
        operation: &str,
        tool: &str,
        path: Option<&Path>,
        failure: &PlatformError,
        metrics: Option<PlatformOperationMetrics>,
    ) {
        let context = Self::tool_operation_context(operation, tool, path);
        self.emit_event(AppEvent::Platform(PlatformEvent::OperationFailed {
            context,
            failure: FailureContext::from_error(failure),
            metrics,
        }));
    }

    /// Set event sender for tool discovery notifications
    pub fn set_event_sender(&self, tx: EventSender) {
        let mut event_tx = self.event_tx.write().unwrap();
        *event_tx = Some(tx);
    }

    /// Load tools from persistent cache
    pub async fn load_from_cache(&self) -> Result<(), PlatformError> {
        if let Some(cache) = PlatformCache::load().await? {
            let cache_path = PlatformCache::default_path()?;
            let search_paths = vec![cache_path.clone()];
            self.emit_tool_operation_started("cache_load", "cache", Some(&cache_path));

            // Process cached tools and collect valid ones
            let mut valid_tools = Vec::new();

            for (name, path_str) in cache.tools {
                let path = PathBuf::from(path_str);

                // Verify tool still exists at cached location
                if path.exists() && self.is_executable(&path) {
                    let version = self.get_tool_version(&path).await.ok();
                    let metadata = self.get_tool_metadata(&name);

                    let cached_tool = CachedTool {
                        path,
                        verified_at: SystemTime::now(),
                        version,
                        metadata,
                    };

                    valid_tools.push((name, cached_tool));
                }
            }

            // Now update the cache with all valid tools at once
            {
                let mut tools = self.tools.write().unwrap();
                for (name, cached_tool) in valid_tools {
                    tools.insert(name, cached_tool);
                }
            }

            let tools_count = {
                let tools = self.tools.read().unwrap();
                tools.len()
            };

            let notes = vec![format!("loaded_tools={tools_count}")];
            self.emit_tool_operation_completed(
                "cache_load",
                "cache",
                Some(&cache_path),
                None,
                &search_paths,
                notes,
            );
        }

        Ok(())
    }

    /// Save current tools to persistent cache
    pub async fn save_to_cache(&self) -> Result<(), PlatformError> {
        // Clone the tools data to avoid holding the lock across await points
        let cache_tools = {
            let tools = self.tools.read().unwrap();
            tools
                .iter()
                .map(|(name, cached_tool)| (name.clone(), cached_tool.path.display().to_string()))
                .collect::<HashMap<String, String>>()
        };

        // Create or load existing cache to preserve user overrides
        let mut cache = PlatformCache::load().await?.unwrap_or_else(|| {
            PlatformCache::new("macos".to_string(), PlatformCapabilities::default())
        });

        // Update with discovered tools (preserving any user overrides)
        for (name, path) in cache_tools {
            cache.tools.insert(name, path);
        }

        cache.discovered_at = chrono::Utc::now().to_rfc3339();
        cache.save().await?;

        Ok(())
    }

    /// Get a tool path, using persistent cache or discovering if necessary
    pub async fn get_tool(&self, name: &str) -> Result<PathBuf, PlatformError> {
        // Check in-memory cache first
        if let Some(cached) = self.get_cached_tool(name) {
            // No TTL check - persistent cache is valid until tool moves
            if cached.path.exists() && self.is_executable(&cached.path) {
                return Ok(cached.path);
            } else {
                // Tool moved or deleted - remove from cache
                self.remove_cached_tool(name);
            }
        }

        // Tool not in memory cache or invalid - discover and cache
        let path = self.discover_tool(name).await?;
        self.cache_tool(name, path.clone()).await;

        // Save to persistent cache for future process starts
        if let Err(e) = self.save_to_cache().await {
            // Don't fail the operation if cache save fails
            let metrics = Self::tool_operation_metrics(None, &[], vec![format!("error={e}")]);
            self.emit_tool_operation_failed("cache_save", "cache", None, &e, metrics);
        }

        Ok(path)
    }

    /// Verify that a set of tools are available
    pub async fn verify_tools(&self, tools: &[&str]) -> Result<(), PlatformError> {
        let mut missing_tools = Vec::new();

        for &tool in tools {
            if self.get_tool(tool).await.is_err() {
                missing_tools.push(tool);
            }
        }

        if !missing_tools.is_empty() {
            let suggestions = missing_tools
                .iter()
                .map(|&tool| self.get_tool_metadata(tool).install_suggestion)
                .collect();
            return Err(PlatformError::MultipleToolsNotFound {
                tools: missing_tools.into_iter().map(String::from).collect(),
                suggestions,
            });
        }

        Ok(())
    }

    /// Get cached tool if available and not expired
    fn get_cached_tool(&self, name: &str) -> Option<CachedTool> {
        let tools = self.tools.read().unwrap();
        tools.get(name).cloned()
    }

    /// Remove a tool from the cache
    fn remove_cached_tool(&self, name: &str) {
        let mut tools = self.tools.write().unwrap();
        tools.remove(name);
    }

    /// Discover a tool by searching paths
    async fn discover_tool(&self, name: &str) -> Result<PathBuf, PlatformError> {
        let search_paths = self.get_search_paths_for_tool(name);
        let start = Instant::now();
        self.emit_tool_operation_started("discover", name, None);

        // Try PATH first (fastest)
        if let Ok(path) = self.find_in_path(name).await {
            let duration = Some(start.elapsed());
            let notes = vec!["source=path".to_string()];
            self.emit_tool_operation_completed(
                "discover",
                name,
                Some(&path),
                duration,
                &search_paths,
                notes,
            );
            return Ok(path);
        }

        // Try fallback paths
        for search_path in search_paths.iter() {
            let candidate = search_path.join(name);
            if candidate.exists() && self.is_executable(&candidate) {
                let duration = Some(start.elapsed());
                let notes = vec![format!("source=fallback:{}", search_path.display())];
                self.emit_tool_operation_completed(
                    "discover",
                    name,
                    Some(&candidate),
                    duration,
                    &search_paths,
                    notes,
                );
                return Ok(candidate);
            }
        }

        // Tool not found
        let metadata = self.get_tool_metadata(name);
        let suggestion = metadata.install_suggestion;
        let error = PlatformError::ToolNotFound {
            tool: name.to_string(),
            suggestion: suggestion.clone(),
            searched_paths: search_paths.clone(),
        };

        let duration = Some(start.elapsed());
        let notes = vec![format!("suggestion={suggestion}")];
        let metrics = Self::tool_operation_metrics(duration, &search_paths, notes);
        self.emit_tool_operation_failed("discover", name, None, &error, metrics);

        Err(error)
    }

    /// Find tool using the system PATH
    async fn find_in_path(&self, name: &str) -> Result<PathBuf, PlatformError> {
        let output = tokio::process::Command::new("which")
            .arg(name)
            .output()
            .await
            .map_err(|e| PlatformError::CommandFailed {
                command: "which".to_string(),
                error: e.to_string(),
            })?;

        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout);
            let path_str = path_str.trim();
            Ok(PathBuf::from(path_str))
        } else {
            Err(PlatformError::ToolNotFound {
                tool: name.to_string(),
                suggestion: "Tool not found in PATH".to_string(),
                searched_paths: vec![],
            })
        }
    }

    /// Get search paths for a specific tool (includes fallbacks)
    fn get_search_paths_for_tool(&self, name: &str) -> Vec<PathBuf> {
        let mut paths = self.search_paths.clone();

        if let Some(fallback_paths) = self.fallback_paths.get(name) {
            for path in fallback_paths {
                if !paths.contains(path) {
                    paths.push(path.clone());
                }
            }
        }

        paths
    }

    /// Check if a file is executable
    fn is_executable(&self, path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;

        if let Ok(metadata) = std::fs::metadata(path) {
            let permissions = metadata.permissions();
            permissions.mode() & 0o111 != 0 // Check execute bits
        } else {
            false
        }
    }

    /// Cache tool information
    async fn cache_tool(&self, name: &str, path: PathBuf) {
        let version = self.get_tool_version(&path).await.ok();
        let metadata = self.get_tool_metadata(name);

        let cached_tool = CachedTool {
            path,
            verified_at: SystemTime::now(),
            version,
            metadata,
        };

        let mut tools = self.tools.write().unwrap();
        tools.insert(name.to_string(), cached_tool);
    }

    /// Get tool version if possible
    async fn get_tool_version(&self, path: &Path) -> Result<String, PlatformError> {
        // Try common version flags
        for flag in &["--version", "-V", "-version"] {
            if let Ok(output) = tokio::process::Command::new(path).arg(flag).output().await {
                if output.status.success() {
                    let version_output = String::from_utf8_lossy(&output.stdout);
                    if let Some(first_line) = version_output.lines().next() {
                        return Ok(first_line.to_string());
                    }
                }
            }
        }

        Err(PlatformError::CommandFailed {
            command: path.display().to_string(),
            error: "Could not determine version".to_string(),
        })
    }

    /// Get metadata for a tool
    fn get_tool_metadata(&self, name: &str) -> ToolMetadata {
        match name {
            // Platform-critical tools
            "otool" | "install_name_tool" | "codesign" => ToolMetadata {
                category: ToolCategory::PlatformCritical,
                is_critical: true,
                install_suggestion: "Install Xcode Command Line Tools: xcode-select --install"
                    .to_string(),
            },

            // Build system tools
            "make" => ToolMetadata {
                category: ToolCategory::BuildSystem,
                is_critical: false,
                install_suggestion: "Install via sps2: sps2 install make".to_string(),
            },
            "cmake" => ToolMetadata {
                category: ToolCategory::BuildSystem,
                is_critical: false,
                install_suggestion: "Install via sps2: sps2 install cmake".to_string(),
            },
            "gcc" => ToolMetadata {
                category: ToolCategory::BuildSystem,
                is_critical: false,
                install_suggestion: "Install via sps2: sps2 install gcc".to_string(),
            },
            "clang" => ToolMetadata {
                category: ToolCategory::BuildSystem,
                is_critical: false,
                install_suggestion: "Install via sps2: sps2 install llvm".to_string(),
            },

            // Development tools
            "configure" | "autoconf" | "automake" => ToolMetadata {
                category: ToolCategory::Development,
                is_critical: false,
                install_suggestion: format!("Install via sps2: sps2 install {name}"),
            },
            "libtool" => ToolMetadata {
                category: ToolCategory::Development,
                is_critical: false,
                install_suggestion: "Install via sps2: sps2 install libtool".to_string(),
            },
            "pkg-config" => ToolMetadata {
                category: ToolCategory::Development,
                is_critical: false,
                install_suggestion: "Install via sps2: sps2 install pkgconf".to_string(),
            },
            "ninja" => ToolMetadata {
                category: ToolCategory::BuildSystem,
                is_critical: false,
                install_suggestion: "Install via sps2: sps2 install ninja".to_string(),
            },
            "meson" => ToolMetadata {
                category: ToolCategory::BuildSystem,
                is_critical: false,
                install_suggestion: "Install via sps2: sps2 install meson".to_string(),
            },

            // System tools
            "which" => ToolMetadata {
                category: ToolCategory::System,
                is_critical: true,
                install_suggestion: "System tool 'which' should be available by default"
                    .to_string(),
            },

            // Unknown tools
            _ => ToolMetadata {
                category: ToolCategory::Development,
                is_critical: false,
                install_suggestion: format!("Tool '{name}' not found. Try: sps2 search {name}"),
            },
        }
    }

    /// Emit an event if sender is available
    fn emit_event(&self, event: AppEvent) {
        let event_tx = self.event_tx.read().unwrap();
        if let Some(tx) = event_tx.as_ref() {
            tx.emit(event);
        }
    }

    /// Get all cached tools for debugging
    pub fn get_cached_tools(&self) -> HashMap<String, CachedTool> {
        self.tools.read().unwrap().clone()
    }

    /// Clear the tool cache (useful for testing or forced refresh)
    pub fn clear_cache(&self) {
        self.tools.write().unwrap().clear();
    }

    /// Get tools by category
    pub async fn get_tools_by_category(&self, category: ToolCategory) -> Vec<String> {
        let all_tools = [
            "otool",
            "install_name_tool",
            "codesign",
            "which",
            "make",
            "cmake",
            "gcc",
            "clang",
            "configure",
            "autoconf",
            "automake",
            "libtool",
            "pkg-config",
            "ninja",
            "meson",
        ];

        all_tools
            .into_iter()
            .filter(|&tool| self.get_tool_metadata(tool).category == category)
            .map(String::from)
            .collect()
    }
}

/// Singleton platform manager for optimized platform operations
pub struct PlatformManager {
    platform: Arc<Platform>,
    capabilities: Arc<PlatformCapabilities>,
    tool_registry: Arc<ToolRegistry>,
}

impl PlatformManager {
    /// Get the singleton instance of the platform manager
    pub fn instance() -> &'static Self {
        static INSTANCE: OnceLock<PlatformManager> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            let platform = Arc::new(Platform::new_internal());
            let capabilities = Arc::new(PlatformCapabilities::default());
            let tool_registry = Arc::new(ToolRegistry::new());

            Self {
                platform,
                capabilities,
                tool_registry,
            }
        })
    }

    /// Get the shared platform instance
    pub fn platform(&self) -> &Arc<Platform> {
        &self.platform
    }

    /// Get platform capabilities
    pub fn capabilities(&self) -> &Arc<PlatformCapabilities> {
        &self.capabilities
    }

    /// Get the tool registry
    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.tool_registry
    }

    /// Get a tool path, discovering and caching if necessary
    pub async fn get_tool(&self, name: &str) -> Result<PathBuf, PlatformError> {
        self.tool_registry.get_tool(name).await
    }

    /// Verify that a set of tools are available
    pub async fn verify_tools(&self, tools: &[&str]) -> Result<(), PlatformError> {
        self.tool_registry.verify_tools(tools).await
    }

    /// Set event sender for tool discovery notifications
    pub fn set_tool_event_sender(&self, tx: EventSender) {
        self.tool_registry.set_event_sender(tx);
    }

    /// Initialize the tool cache from persistent storage
    /// This should be called once during application startup
    pub async fn initialize_cache(&self) -> Result<(), PlatformError> {
        self.tool_registry.load_from_cache().await
    }

    /// Save current tool cache to persistent storage
    pub async fn save_cache(&self) -> Result<(), PlatformError> {
        self.tool_registry.save_to_cache().await
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

    /// Internal method to create platform instance for singleton
    fn new_internal() -> Self {
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

    // REMOVED: Use PlatformManager::instance().platform() instead

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
    pub fn create_context(&self, event_sender: Option<EventSender>) -> PlatformContext {
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
        event_sender: Option<EventSender>,
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
