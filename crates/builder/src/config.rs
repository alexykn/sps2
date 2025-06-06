//! Build configuration for package building

use crate::monitoring::MonitoringConfig;
use crate::quality_assurance::QaConfig;
use crate::{CompressionConfig, SbomConfig, SigningConfig};
use std::path::PathBuf;

/// Package builder configuration
#[derive(Clone, Debug)]
pub struct BuildConfig {
    /// SBOM generation configuration
    pub sbom_config: SbomConfig,
    /// Package signing configuration
    pub signing_config: SigningConfig,
    /// Maximum build time in seconds
    pub max_build_time: Option<u64>,
    /// Network access allowed during build
    pub allow_network: bool,
    /// Number of parallel build jobs
    pub build_jobs: Option<usize>,
    /// Build root directory (defaults to current directory)
    pub build_root: Option<PathBuf>,
    /// Compression configuration for package archives
    pub compression_config: CompressionConfig,
    /// Monitoring configuration
    pub monitoring_config: MonitoringConfig,
    /// Quality assurance configuration
    pub qa_config: QaConfig,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            sbom_config: SbomConfig::default(),
            signing_config: SigningConfig::default(),
            max_build_time: Some(3600), // 1 hour
            allow_network: false,
            build_jobs: None, // Use auto-detection
            build_root: Some(PathBuf::from("/opt/pm/build")), // Default to /opt/pm/build
            compression_config: CompressionConfig::default(),
            monitoring_config: MonitoringConfig::default(),
            qa_config: QaConfig::default(),
        }
    }
}

impl BuildConfig {
    /// Create config with network access enabled
    #[must_use]
    pub fn with_network() -> Self {
        Self {
            allow_network: true,
            ..Default::default()
        }
    }

    /// Set SBOM configuration
    #[must_use]
    pub fn with_sbom_config(mut self, config: SbomConfig) -> Self {
        self.sbom_config = config;
        self
    }

    /// Set signing configuration
    #[must_use]
    pub fn with_signing_config(mut self, config: SigningConfig) -> Self {
        self.signing_config = config;
        self
    }

    /// Set build timeout
    #[must_use]
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.max_build_time = Some(seconds);
        self
    }

    /// Set parallel build jobs
    #[must_use]
    pub fn with_jobs(mut self, jobs: usize) -> Self {
        self.build_jobs = Some(jobs);
        self
    }

    /// Set compression configuration
    #[must_use]
    pub fn with_compression_config(mut self, config: CompressionConfig) -> Self {
        self.compression_config = config;
        self
    }

    /// Set compression level
    #[must_use]
    pub fn with_compression_level(mut self, level: crate::CompressionLevel) -> Self {
        self.compression_config.level = level;
        self
    }

    /// Enable fast compression for development builds
    #[must_use]
    pub fn with_fast_compression() -> Self {
        Self {
            compression_config: CompressionConfig::fast(),
            ..Default::default()
        }
    }

    /// Enable balanced compression (default)
    #[must_use]
    pub fn with_balanced_compression() -> Self {
        Self {
            compression_config: CompressionConfig::balanced(),
            ..Default::default()
        }
    }

    /// Enable maximum compression for production builds
    #[must_use]
    pub fn with_maximum_compression() -> Self {
        Self {
            compression_config: CompressionConfig::maximum(),
            ..Default::default()
        }
    }

    /// Enable custom compression level
    #[must_use]
    pub fn with_custom_compression(level: u8) -> Self {
        Self {
            compression_config: CompressionConfig::custom(level),
            ..Default::default()
        }
    }

    /// Set monitoring configuration
    #[must_use]
    pub fn with_monitoring_config(mut self, config: MonitoringConfig) -> Self {
        self.monitoring_config = config;
        self
    }

    /// Enable monitoring with default settings
    #[must_use]
    pub fn with_monitoring(mut self) -> Self {
        self.monitoring_config = MonitoringConfig::enabled();
        self
    }

    /// Enable development monitoring (detailed)
    #[must_use]
    pub fn with_development_monitoring(mut self) -> Self {
        self.monitoring_config = MonitoringConfig::development();
        self
    }

    /// Set quality assurance configuration
    #[must_use]
    pub fn with_qa_config(mut self, config: QaConfig) -> Self {
        self.qa_config = config;
        self
    }

    /// Enable minimal QA checks (for development)
    #[must_use]
    pub fn with_minimal_qa(mut self) -> Self {
        self.qa_config = QaConfig::minimal();
        self
    }

    /// Enable standard QA checks
    #[must_use]
    pub fn with_standard_qa(mut self) -> Self {
        self.qa_config = QaConfig::standard();
        self
    }

    /// Enable strict QA checks (for releases)
    #[must_use]
    pub fn with_strict_qa(mut self) -> Self {
        self.qa_config = QaConfig::strict();
        self
    }

    /// Disable QA checks
    #[must_use]
    pub fn without_qa(mut self) -> Self {
        self.qa_config.flags = crate::quality_assurance::config::QaComponentFlags::empty();
        self
    }

    /// Enable production monitoring (optimized)
    #[must_use]
    pub fn with_prod_monitoring(mut self) -> Self {
        self.monitoring_config = MonitoringConfig::production();
        self
    }

    /// Set build root directory
    #[must_use]
    pub fn with_build_root(mut self, path: PathBuf) -> Self {
        self.build_root = Some(path);
        self
    }
}
