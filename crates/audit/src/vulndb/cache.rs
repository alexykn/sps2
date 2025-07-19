//! Query result caching for performance optimization



use crate::types::Vulnerability;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Cache entry with timestamp for TTL
#[derive(Debug, Clone)]
struct CacheEntry {
    /// Cached vulnerabilities
    vulnerabilities: Vec<Vulnerability>,
    /// When this entry was created
    created_at: Instant,
}

/// Simple in-memory cache for vulnerability queries
///
/// This is a basic implementation that can be extended with:
/// - LRU eviction
/// - Size limits
/// - More sophisticated TTL handling
/// - Persistent cache storage
#[derive(Debug)]
pub struct VulnerabilityCache {
    /// Package name -> vulnerabilities cache
    package_cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// PURL -> vulnerabilities cache
    purl_cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// CPE -> vulnerabilities cache
    cpe_cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// CVE ID -> vulnerability cache
    cve_cache: Arc<RwLock<HashMap<String, Option<Vulnerability>>>>,
    /// Cache TTL (time to live)
    ttl: Duration,
}

impl VulnerabilityCache {
    /// Create new vulnerability cache with specified TTL
    pub fn new(ttl: Duration) -> Self {
        Self {
            package_cache: Arc::new(RwLock::new(HashMap::new())),
            purl_cache: Arc::new(RwLock::new(HashMap::new())),
            cpe_cache: Arc::new(RwLock::new(HashMap::new())),
            cve_cache: Arc::new(RwLock::new(HashMap::new())),
            ttl,
        }
    }

    /// Create cache with default 5 minute TTL
    pub fn with_default_ttl() -> Self {
        Self::new(Duration::from_secs(300)) // 5 minutes
    }

    /// Get cached vulnerabilities for package
    pub fn get_package_vulnerabilities(&self, package_key: &str) -> Option<Vec<Vulnerability>> {
        let cache = self.package_cache.read().ok()?;
        let entry = cache.get(package_key)?;

        if entry.created_at.elapsed() < self.ttl {
            Some(entry.vulnerabilities.clone())
        } else {
            None
        }
    }

    /// Cache vulnerabilities for package
    pub fn cache_package_vulnerabilities(
        &self,
        package_key: String,
        vulnerabilities: Vec<Vulnerability>,
    ) {
        if let Ok(mut cache) = self.package_cache.write() {
            cache.insert(
                package_key,
                CacheEntry {
                    vulnerabilities,
                    created_at: Instant::now(),
                },
            );
        }
    }

    /// Get cached vulnerabilities for PURL
    pub fn get_purl_vulnerabilities(&self, purl: &str) -> Option<Vec<Vulnerability>> {
        let cache = self.purl_cache.read().ok()?;
        let entry = cache.get(purl)?;

        if entry.created_at.elapsed() < self.ttl {
            Some(entry.vulnerabilities.clone())
        } else {
            None
        }
    }

    /// Cache vulnerabilities for PURL
    pub fn cache_purl_vulnerabilities(&self, purl: String, vulnerabilities: Vec<Vulnerability>) {
        if let Ok(mut cache) = self.purl_cache.write() {
            cache.insert(
                purl,
                CacheEntry {
                    vulnerabilities,
                    created_at: Instant::now(),
                },
            );
        }
    }

    /// Get cached vulnerabilities for CPE
    pub fn get_cpe_vulnerabilities(&self, cpe: &str) -> Option<Vec<Vulnerability>> {
        let cache = self.cpe_cache.read().ok()?;
        let entry = cache.get(cpe)?;

        if entry.created_at.elapsed() < self.ttl {
            Some(entry.vulnerabilities.clone())
        } else {
            None
        }
    }

    /// Cache vulnerabilities for CPE
    pub fn cache_cpe_vulnerabilities(&self, cpe: String, vulnerabilities: Vec<Vulnerability>) {
        if let Ok(mut cache) = self.cpe_cache.write() {
            cache.insert(
                cpe,
                CacheEntry {
                    vulnerabilities,
                    created_at: Instant::now(),
                },
            );
        }
    }

    /// Get cached vulnerability by CVE ID
    #[allow(clippy::option_option)] // Distinguishes not-cached from cached-but-empty
    pub fn get_cve_vulnerability(&self, cve_id: &str) -> Option<Option<Vulnerability>> {
        let cache = self.cve_cache.read().ok()?;
        cache.get(cve_id).cloned()
    }

    /// Cache vulnerability by CVE ID
    pub fn cache_cve_vulnerability(&self, cve_id: String, vulnerability: Option<Vulnerability>) {
        if let Ok(mut cache) = self.cve_cache.write() {
            cache.insert(cve_id, vulnerability);
        }
    }

    /// Clear all caches
    pub fn clear(&self) {
        if let (Ok(mut package_cache), Ok(mut purl_cache), Ok(mut cpe_cache), Ok(mut cve_cache)) = (
            self.package_cache.write(),
            self.purl_cache.write(),
            self.cpe_cache.write(),
            self.cve_cache.write(),
        ) {
            package_cache.clear();
            purl_cache.clear();
            cpe_cache.clear();
            cve_cache.clear();
        }
    }

    /// Remove expired entries from all caches
    pub fn cleanup_expired(&self) {
        self.cleanup_package_cache();
        self.cleanup_purl_cache();
        self.cleanup_cpe_cache();
        // CVE cache doesn't have TTL-based entries in current implementation
    }

    /// Remove expired entries from package cache
    fn cleanup_package_cache(&self) {
        if let Ok(mut cache) = self.package_cache.write() {
            cache.retain(|_, entry| entry.created_at.elapsed() < self.ttl);
        }
    }

    /// Remove expired entries from PURL cache
    fn cleanup_purl_cache(&self) {
        if let Ok(mut cache) = self.purl_cache.write() {
            cache.retain(|_, entry| entry.created_at.elapsed() < self.ttl);
        }
    }

    /// Remove expired entries from CPE cache
    fn cleanup_cpe_cache(&self) {
        if let Ok(mut cache) = self.cpe_cache.write() {
            cache.retain(|_, entry| entry.created_at.elapsed() < self.ttl);
        }
    }

    /// Get cache statistics for monitoring
    pub fn get_cache_stats(&self) -> CacheStatistics {
        let package_count = self
            .package_cache
            .read()
            .map(|cache| cache.len())
            .unwrap_or(0);
        let purl_count = self.purl_cache.read().map(|cache| cache.len()).unwrap_or(0);
        let cpe_count = self.cpe_cache.read().map(|cache| cache.len()).unwrap_or(0);
        let cve_entries_count = self.cve_cache.read().map(|cache| cache.len()).unwrap_or(0);

        CacheStatistics {
            package_entries: package_count,
            purl_entries: purl_count,
            cpe_entries: cpe_count,
            cve_entries: cve_entries_count,
            ttl_seconds: self.ttl.as_secs(),
        }
    }
}

/// Cache statistics for monitoring and debugging
#[derive(Debug, Clone)]
pub struct CacheStatistics {
    /// Number of package cache entries
    pub package_entries: usize,
    /// Number of PURL cache entries
    pub purl_entries: usize,
    /// Number of CPE cache entries
    pub cpe_entries: usize,
    /// Number of CVE cache entries
    pub cve_entries: usize,
    /// TTL in seconds
    pub ttl_seconds: u64,
}
