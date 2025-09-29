/// Installer configuration
#[derive(Clone, Debug)]
pub struct InstallConfig {
    /// Maximum concurrent downloads
    pub max_concurrency: usize,
    /// Download timeout in seconds
    pub download_timeout: u64,
    /// Enable APFS optimizations
    pub enable_apfs: bool,
    /// State retention policy (number of states to keep)
    pub state_retention: usize,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            download_timeout: 300, // 5 minutes
            enable_apfs: cfg!(target_os = "macos"),
            state_retention: 10,
        }
    }
}

impl InstallConfig {
    /// Create config with custom concurrency
    #[must_use]
    pub fn with_concurrency(mut self, max_concurrency: usize) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    /// Set download timeout
    #[must_use]
    pub fn with_timeout(mut self, timeout_seconds: u64) -> Self {
        self.download_timeout = timeout_seconds;
        self
    }

    /// Enable/disable APFS optimizations
    #[must_use]
    pub fn with_apfs(mut self, enable: bool) -> Self {
        self.enable_apfs = enable;
        self
    }

    /// Set state retention policy
    #[must_use]
    pub fn with_retention(mut self, count: usize) -> Self {
        self.state_retention = count;
        self
    }
}

/// Security policy for signature enforcement
#[derive(Clone, Copy, Debug)]
pub struct SecurityPolicy {
    pub verify_signatures: bool,
    pub allow_unsigned: bool,
}
