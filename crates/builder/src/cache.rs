// Crate-level pedantic settings apply

//! Caching and incremental builds system
//!
//! This module provides build caching, artifact storage, and incremental build tracking
//! to speed up repeated builds and avoid unnecessary recompilation.

use sps2_errors::Error;
use sps2_events::{AppEvent, BuildDiagnostic, BuildEvent, EventEmitter, EventSender};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::fs;
use tokio::sync::RwLock;

/// Build cache for compiler caching and source downloads
#[derive(Debug, Clone)]
pub struct BuildCache {
    cache_root: PathBuf,
    source_cache: Arc<SourceCache>,
    compiler_cache: Arc<CompilerCache>,
    stats: Arc<RwLock<CacheStatistics>>,
    event_sender: Option<sps2_events::EventSender>,
}

impl EventEmitter for BuildCache {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}
impl BuildCache {
    /// Create a new build cache
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Failed to create the cache directory
    /// - Failed to initialize the source or compiler caches
    pub async fn new(
        cache_root: PathBuf,
        event_sender: Option<EventSender>,
    ) -> Result<Self, Error> {
        fs::create_dir_all(&cache_root).await?;

        let source_cache = Arc::new(SourceCache::new(cache_root.join("sources")).await?);
        let compiler_cache = Arc::new(CompilerCache::new(cache_root.join("compiler")).await?);
        let stats = Arc::new(RwLock::new(CacheStatistics::default()));

        Ok(Self {
            cache_root,
            source_cache,
            compiler_cache,
            stats,
            event_sender,
        })
    }

    /// Get cache statistics
    pub async fn get_statistics(&self) -> CacheStatistics {
        self.stats.read().await.clone()
    }

    /// Get the cache root directory
    #[must_use]
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    /// Get the compiler cache instance
    #[must_use]
    pub fn compiler_cache(&self) -> &CompilerCache {
        &self.compiler_cache
    }

    /// Get the source cache instance
    #[must_use]
    pub fn source_cache(&self) -> &SourceCache {
        &self.source_cache
    }

    /// Clear all caches
    ///
    /// # Errors
    ///
    /// Returns an error if cache cleanup fails.
    pub async fn clear_all(&self) -> Result<(), Error> {
        self.source_cache.clear().await;

        // Reset statistics
        let mut stats = self.stats.write().await;
        *stats = CacheStatistics::default();

        self.emit(AppEvent::Build(BuildEvent::Diagnostic(
            BuildDiagnostic::CachePruned {
                removed_items: 0,
                freed_bytes: 0,
            },
        )));

        Ok(())
    }
}

/// Simple source cache for downloads and git repositories
#[derive(Debug)]
pub struct SourceCache {
    #[allow(dead_code)] // Stored for potential future cache operations
    cache_dir: PathBuf,
    downloads: RwLock<HashMap<String, PathBuf>>,
    git_repos: RwLock<HashMap<String, PathBuf>>,
}

impl SourceCache {
    /// Create new source cache
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created.
    pub async fn new(cache_dir: PathBuf) -> Result<Self, Error> {
        fs::create_dir_all(&cache_dir).await?;
        Ok(Self {
            cache_dir,
            downloads: RwLock::new(HashMap::new()),
            git_repos: RwLock::new(HashMap::new()),
        })
    }

    /// Cache a downloaded file
    pub async fn cache_download(&self, url: String, path: PathBuf) {
        let mut downloads = self.downloads.write().await;
        downloads.insert(url, path);
    }

    /// Get cached download path
    pub async fn get_download(&self, url: &str) -> Option<PathBuf> {
        let downloads = self.downloads.read().await;
        downloads.get(url).cloned()
    }

    /// Cache a git repository
    pub async fn cache_git_repo(&self, url: String, path: PathBuf) {
        let mut repos = self.git_repos.write().await;
        repos.insert(url, path);
    }

    /// Get cached git repository path
    pub async fn get_git_repo(&self, url: &str) -> Option<PathBuf> {
        let repos = self.git_repos.read().await;
        repos.get(url).cloned()
    }

    /// Clear all cached entries
    pub async fn clear(&self) {
        let mut downloads = self.downloads.write().await;
        let mut repos = self.git_repos.write().await;
        downloads.clear();
        repos.clear();
    }
}

/// Compiler cache integration
#[derive(Debug)]
pub struct CompilerCache {
    cache_type: CompilerCacheType,
    cache_dir: PathBuf,
    max_size: u64,
}

/// Type of compiler cache
#[derive(Debug, Clone, Copy)]
pub enum CompilerCacheType {
    /// ccache
    CCache,
    /// sccache
    SCCache,
    /// No cache
    None,
}

impl CompilerCache {
    async fn new(cache_dir: PathBuf) -> Result<Self, Error> {
        fs::create_dir_all(&cache_dir).await?;

        // Detect available compiler cache
        let cache_type = if which::which("sccache").is_ok() {
            CompilerCacheType::SCCache
        } else if which::which("ccache").is_ok() {
            CompilerCacheType::CCache
        } else {
            CompilerCacheType::None
        };

        Ok(Self {
            cache_type,
            cache_dir,
            max_size: 5 * 1024 * 1024 * 1024, // 5GB default
        })
    }

    /// Get environment variables for compiler cache
    ///
    /// Returns environment variables that should be set for the compiler cache to work.
    /// These variables configure cache directories and size limits.
    #[must_use]
    pub fn get_env_vars(&self) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        match self.cache_type {
            CompilerCacheType::CCache => {
                vars.insert(
                    "CCACHE_DIR".to_string(),
                    self.cache_dir.display().to_string(),
                );
                vars.insert(
                    "CCACHE_MAXSIZE".to_string(),
                    format!("{}G", self.max_size / (1024 * 1024 * 1024)),
                );
            }
            CompilerCacheType::SCCache => {
                vars.insert(
                    "SCCACHE_DIR".to_string(),
                    self.cache_dir.display().to_string(),
                );
                vars.insert(
                    "SCCACHE_CACHE_SIZE".to_string(),
                    format!("{}G", self.max_size / (1024 * 1024 * 1024)),
                );
                vars.insert("RUSTC_WRAPPER".to_string(), "sccache".to_string());
            }
            CompilerCacheType::None => {}
        }

        vars
    }

    /// Get wrapper command for compiler
    ///
    /// Returns the wrapper command (ccache or sccache) if a compiler cache is enabled.
    /// This should be prepended to compiler invocations.
    #[must_use]
    pub fn get_wrapper(&self) -> Option<&'static str> {
        match self.cache_type {
            CompilerCacheType::CCache => Some("ccache"),
            CompilerCacheType::SCCache => Some("sccache"),
            CompilerCacheType::None => None,
        }
    }
}

/// Incremental build tracking
#[derive(Debug)]
pub struct IncrementalBuildTracker {
    /// File modification times
    file_mtimes: RwLock<HashMap<PathBuf, SystemTime>>,
    /// Dependency graph
    dep_graph: RwLock<HashMap<PathBuf, Vec<PathBuf>>>,
    /// Changed files
    changed_files: RwLock<Vec<PathBuf>>,
}

impl IncrementalBuildTracker {
    /// Create new incremental build tracker
    ///
    /// The tracker must be used to monitor file changes and determine if rebuilds are needed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            file_mtimes: RwLock::new(HashMap::new()),
            dep_graph: RwLock::new(HashMap::new()),
            changed_files: RwLock::new(Vec::new()),
        }
    }

    /// Track file modification
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Failed to access file metadata
    /// - I/O errors occur while reading file information
    pub async fn track_file(&self, path: &Path) -> Result<(), Error> {
        if let Ok(metadata) = fs::metadata(path).await {
            if let Ok(mtime) = metadata.modified() {
                let mut mtimes = self.file_mtimes.write().await;
                mtimes.insert(path.to_path_buf(), mtime);
            }
        }
        Ok(())
    }

    /// Check if file has changed
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Failed to access file metadata
    /// - I/O errors occur while reading file information
    pub async fn has_file_changed(&self, path: &Path) -> Result<bool, Error> {
        let mtimes = self.file_mtimes.read().await;

        if let Some(&stored_mtime) = mtimes.get(path) {
            if let Ok(metadata) = fs::metadata(path).await {
                if let Ok(current_mtime) = metadata.modified() {
                    return Ok(current_mtime > stored_mtime);
                }
            }
        }

        // If we can't determine, assume it changed
        Ok(true)
    }

    /// Add dependency relationship
    pub async fn add_dependency(&self, target: PathBuf, dependency: PathBuf) {
        let mut graph = self.dep_graph.write().await;
        graph.entry(target).or_default().push(dependency);
    }

    /// Get files that need rebuilding
    pub async fn get_files_to_rebuild(&self) -> Vec<PathBuf> {
        self.changed_files.read().await.clone()
    }

    /// Mark file as changed
    pub async fn mark_changed(&self, path: PathBuf) {
        let mut changed = self.changed_files.write().await;
        if !changed.contains(&path) {
            changed.push(path);
        }
    }

    /// Clear changed files after build
    pub async fn clear_changed_files(&self) {
        self.changed_files.write().await.clear();
    }
}

impl Default for IncrementalBuildTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics
#[derive(Debug, Clone, Default)]
pub struct CacheStatistics {
    /// Number of cache hits
    pub cache_hits: u64,
    /// Number of cache misses
    pub cache_misses: u64,
    /// Number of artifacts cached
    pub artifacts_cached: u64,
    /// Number of evictions
    pub evictions: u64,
}

impl CacheStatistics {
    /// Get cache hit rate
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // Acceptable for percentage calculation
    pub fn hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            (self.cache_hits as f64 / total as f64) * 100.0
        }
    }
}
