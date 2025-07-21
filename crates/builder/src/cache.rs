// Crate-level pedantic settings apply

//! Caching and incremental builds system
//!
//! This module provides build caching, artifact storage, and incremental build tracking
//! to speed up repeated builds and avoid unnecessary recompilation.

use sps2_errors::Error;
use sps2_events::{Event, EventEmitter, EventSender};
use sps2_hash::Hash;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::fs;
use tokio::sync::RwLock;

/// Build cache for storing and retrieving build artifacts
#[derive(Debug, Clone)]
pub struct BuildCache {
    cache_root: PathBuf,
    store: Arc<ContentAddressedStore>,
    artifact_cache: Arc<ArtifactCache>,
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
    /// - Failed to initialize the content-addressed store
    /// - Failed to initialize the artifact or compiler caches
    pub async fn new(
        cache_root: PathBuf,
        event_sender: Option<EventSender>,
    ) -> Result<Self, Error> {
        fs::create_dir_all(&cache_root).await?;

        let store = Arc::new(ContentAddressedStore::new(cache_root.join("store")).await?);
        let artifact_cache = Arc::new(ArtifactCache::new(cache_root.join("artifacts")).await?);
        let compiler_cache = Arc::new(CompilerCache::new(cache_root.join("compiler")).await?);
        let stats = Arc::new(RwLock::new(CacheStatistics::default()));

        Ok(Self {
            cache_root,
            store,
            artifact_cache,
            compiler_cache,
            stats,
            event_sender,
        })
    }

    /// Cache build artifacts with proper invalidation
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Failed to store artifacts in the artifact cache
    /// - I/O operations fail while writing to cache
    pub async fn cache_artifacts(
        &self,
        artifacts: Vec<Artifact>,
        cache_key: &CacheKey,
    ) -> Result<(), Error> {
        let mut stats = self.stats.write().await;

        for artifact in artifacts {
            // Store artifact with content addressing
            let hash = artifact.compute_hash().await?;
            self.store.store(&artifact.path, &hash).await?;

            // Track artifact in cache
            self.artifact_cache
                .store_artifact(cache_key, artifact, hash)
                .await?;

            stats.artifacts_cached += 1;
        }

        // Send cache event
        self.emit_event(Event::BuildCacheUpdated {
            cache_key: cache_key.to_string(),
            artifacts_count: stats.artifacts_cached.try_into().unwrap_or(usize::MAX),
        });

        Ok(())
    }

    /// Retrieve cached artifacts if valid
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Failed to read from the artifact cache
    /// - Cache metadata is corrupted
    /// - I/O operations fail while accessing cache
    pub async fn get_cached_artifacts(
        &self,
        cache_key: &CacheKey,
    ) -> Result<Option<Vec<Artifact>>, Error> {
        let mut stats = self.stats.write().await;

        // Check if cache entry exists
        if let Some(artifacts) = self.artifact_cache.get_artifacts(cache_key).await? {
            // Verify all artifacts are still valid
            let mut valid_artifacts = Vec::new();

            for artifact in artifacts {
                if self.store.exists(&artifact.hash).await? {
                    valid_artifacts.push(artifact);
                } else {
                    // Cache entry is stale
                    stats.cache_misses += 1;
                    self.emit_event(Event::BuildCacheMiss {
                        cache_key: cache_key.to_string(),
                        reason: "Artifact missing from store".to_string(),
                    });
                    return Ok(None);
                }
            }

            stats.cache_hits += 1;
            self.emit_event(Event::BuildCacheHit {
                cache_key: cache_key.to_string(),
                artifacts_count: valid_artifacts.len(),
            });

            Ok(Some(valid_artifacts))
        } else {
            stats.cache_misses += 1;
            self.emit_event(Event::BuildCacheMiss {
                cache_key: cache_key.to_string(),
                reason: "No cache entry found".to_string(),
            });
            Ok(None)
        }
    }

    /// Generate cache key from inputs
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Failed to compute hash for build inputs
    /// - Failed to read source files for hashing
    pub async fn generate_cache_key(&self, inputs: &BuildInputs) -> Result<CacheKey, Error> {
        CacheKey::generate(inputs).await
    }

    /// Get cache statistics
    pub async fn get_statistics(&self) -> CacheStatistics {
        self.stats.read().await.clone()
    }

    /// Clean cache based on LRU policy
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Failed to get cache size information
    /// - Failed to remove cache entries
    /// - I/O operations fail during cleanup
    pub async fn clean_cache(&self, max_size: u64) -> Result<(), Error> {
        let mut stats = self.stats.write().await;

        // Get current cache size
        let current_size = self.store.get_total_size().await?;

        if current_size > max_size {
            // Remove least recently used items
            let removed = self.store.evict_lru(current_size - max_size).await?;
            stats.evictions += removed as u64;

            self.emit_event(Event::BuildCacheCleaned {
                removed_items: removed,
                freed_bytes: current_size - self.store.get_total_size().await?,
            });
        }

        Ok(())
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
}

/// Cache key for identifying build artifacts
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// Hash of source files
    source_hash: Hash,
    /// Compiler version
    compiler_version: String,
    /// Build flags
    flags: Vec<String>,
    /// Environment variables that affect build
    env_hash: Hash,
    /// Platform triple
    platform: String,
    /// Dependency versions
    deps_hash: Hash,
}

impl CacheKey {
    /// Generate cache key from build inputs
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Failed to read source files for hashing
    /// - Failed to compute hash for inputs
    /// - I/O errors occur while accessing files
    pub async fn generate(inputs: &BuildInputs) -> Result<Self, Error> {
        // Hash source files
        let mut source_data = Vec::new();
        for path in &inputs.source_files {
            if path.exists() {
                let content = fs::read(path).await?;
                source_data.extend_from_slice(&content);
            }
        }
        let source_hash = Hash::from_data(&source_data);

        // Hash environment variables
        let mut env_data = Vec::new();
        for (key, value) in &inputs.env_vars {
            env_data.extend_from_slice(key.as_bytes());
            env_data.extend_from_slice(b"=");
            env_data.extend_from_slice(value.as_bytes());
            env_data.extend_from_slice(b"\n");
        }
        let env_hash = Hash::from_data(&env_data);

        // Hash dependencies
        let mut deps_data = Vec::new();
        for (name, version) in &inputs.dependencies {
            deps_data.extend_from_slice(name.as_bytes());
            deps_data.extend_from_slice(b"@");
            deps_data.extend_from_slice(version.as_bytes());
            deps_data.extend_from_slice(b"\n");
        }
        let deps_hash = Hash::from_data(&deps_data);

        Ok(Self {
            source_hash,
            compiler_version: inputs.compiler_version.clone(),
            flags: inputs.flags.clone(),
            env_hash,
            platform: inputs.platform.clone(),
            deps_hash,
        })
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-{}-{}",
            self.source_hash
                .to_string()
                .chars()
                .take(8)
                .collect::<String>(),
            self.compiler_version,
            self.platform
        )
    }
}

/// Build inputs for cache key generation
#[derive(Debug, Clone)]
pub struct BuildInputs {
    /// Source files to hash
    pub source_files: Vec<PathBuf>,
    /// Compiler version
    pub compiler_version: String,
    /// Build flags
    pub flags: Vec<String>,
    /// Environment variables
    pub env_vars: HashMap<String, String>,
    /// Platform triple
    pub platform: String,
    /// Dependencies with versions
    pub dependencies: HashMap<String, String>,
}

/// Artifact representing a cached build result
#[derive(Debug, Clone)]
pub struct Artifact {
    /// Path to the artifact
    pub path: PathBuf,
    /// Type of artifact
    pub artifact_type: ArtifactType,
    /// Hash of the artifact
    pub hash: Hash,
    /// Size in bytes
    pub size: u64,
    /// Last access time
    pub last_accessed: SystemTime,
}

impl Artifact {
    /// Compute hash of the artifact file
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The artifact file cannot be read
    /// - I/O errors occur while reading the file
    pub async fn compute_hash(&self) -> Result<Hash, Error> {
        Hash::hash_file(&self.path).await
    }
}

/// Type of build artifact
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactType {
    /// Compiled object file
    Object,
    /// Static library
    StaticLibrary,
    /// Dynamic library
    DynamicLibrary,
    /// Executable
    Executable,
    /// Test results
    TestResults,
    /// SBOM file
    Sbom,
    /// Other artifact
    Other(String),
}

/// Content-addressed storage for artifacts
#[derive(Debug)]
struct ContentAddressedStore {
    root: PathBuf,
    index: RwLock<HashMap<Hash, StoreEntry>>,
}

#[derive(Debug, Clone)]
struct StoreEntry {
    path: PathBuf,
    size: u64,
    last_accessed: SystemTime,
}

impl ContentAddressedStore {
    async fn new(root: PathBuf) -> Result<Self, Error> {
        fs::create_dir_all(&root).await?;
        Ok(Self {
            root,
            index: RwLock::new(HashMap::new()),
        })
    }

    async fn store(&self, source: &Path, hash: &Hash) -> Result<(), Error> {
        let dest = self.root.join(hash.to_string());
        if !dest.exists() {
            fs::copy(source, &dest).await?;
        }

        let metadata = fs::metadata(&dest).await?;
        let mut index = self.index.write().await;
        index.insert(
            hash.clone(),
            StoreEntry {
                path: dest,
                size: metadata.len(),
                last_accessed: SystemTime::now(),
            },
        );

        Ok(())
    }

    async fn exists(&self, hash: &Hash) -> Result<bool, Error> {
        let index = self.index.read().await;
        if let Some(entry) = index.get(hash) {
            Ok(entry.path.exists())
        } else {
            Ok(false)
        }
    }

    async fn get_total_size(&self) -> Result<u64, Error> {
        let index = self.index.read().await;
        Ok(index.values().map(|e| e.size).sum())
    }

    async fn evict_lru(&self, bytes_to_free: u64) -> Result<usize, Error> {
        let mut index = self.index.write().await;
        let mut entries: Vec<_> = index.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        entries.sort_by_key(|(_, e)| e.last_accessed);

        let mut freed = 0u64;
        let mut removed = 0;
        let mut to_remove = Vec::new();

        for (hash, entry) in entries {
            if freed >= bytes_to_free {
                break;
            }

            if entry.path.exists() {
                fs::remove_file(&entry.path).await?;
                freed += entry.size;
                removed += 1;
                to_remove.push(hash);
            }
        }

        for hash in to_remove {
            index.remove(&hash);
        }

        Ok(removed)
    }
}

/// Artifact cache for quick lookups
#[derive(Debug)]
struct ArtifactCache {
    #[allow(dead_code)] // Root stored for potential future cache operations
    root: PathBuf,
    index: RwLock<HashMap<CacheKey, Vec<Artifact>>>,
}

impl ArtifactCache {
    async fn new(root: PathBuf) -> Result<Self, Error> {
        fs::create_dir_all(&root).await?;
        Ok(Self {
            root,
            index: RwLock::new(HashMap::new()),
        })
    }

    async fn store_artifact(
        &self,
        key: &CacheKey,
        artifact: Artifact,
        _hash: Hash,
    ) -> Result<(), Error> {
        let mut index = self.index.write().await;
        index.entry(key.clone()).or_default().push(artifact);
        Ok(())
    }

    async fn get_artifacts(&self, key: &CacheKey) -> Result<Option<Vec<Artifact>>, Error> {
        let mut index = self.index.write().await;
        if let Some(artifacts) = index.get_mut(key) {
            // Update last accessed time
            for artifact in artifacts.iter_mut() {
                artifact.last_accessed = SystemTime::now();
            }
            Ok(Some(artifacts.clone()))
        } else {
            Ok(None)
        }
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
