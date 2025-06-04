//! Quality assurance configuration

use super::types::{LinterConfig, PolicyRule, QaSeverity, ScannerConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

bitflags::bitflags! {
    /// Quality assurance component flags
    #[derive(Debug, Clone, Copy)]
    pub struct QaComponentFlags: u8 {
        /// Whether to run linters
        const LINTERS = 0b0001;
        /// Whether to run security scanners
        const SCANNERS = 0b0010;
        /// Whether to run policy validators
        const POLICY_VALIDATORS = 0b0100;
        /// Fail build on warnings
        const FAIL_ON_WARNINGS = 0b1000;
    }
}

impl Default for QaComponentFlags {
    fn default() -> Self {
        Self::LINTERS | Self::SCANNERS | Self::POLICY_VALIDATORS
    }
}

impl QaComponentFlags {
    /// Create flags with minimal QA (linters only)
    #[must_use]
    pub fn minimal() -> Self {
        Self::LINTERS
    }

    /// Create flags with strict QA (all enabled, fail on warnings)
    #[must_use]
    pub fn strict() -> Self {
        Self::LINTERS | Self::SCANNERS | Self::POLICY_VALIDATORS | Self::FAIL_ON_WARNINGS
    }

    /// Check if linters are enabled
    #[must_use]
    pub fn linters(self) -> bool {
        self.contains(Self::LINTERS)
    }

    /// Check if scanners are enabled
    #[must_use]
    pub fn scanners(self) -> bool {
        self.contains(Self::SCANNERS)
    }

    /// Check if policy validators are enabled
    #[must_use]
    pub fn policy_validators(self) -> bool {
        self.contains(Self::POLICY_VALIDATORS)
    }

    /// Check if build should fail on warnings
    #[must_use]
    pub fn fail_on_warnings(self) -> bool {
        self.contains(Self::FAIL_ON_WARNINGS)
    }
}

// Manual serde implementation for bitflags
impl serde::Serialize for QaComponentFlags {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u8(self.bits())
    }
}

impl<'de> serde::Deserialize<'de> for QaComponentFlags {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bits = u8::deserialize(deserializer)?;
        Self::from_bits(bits).ok_or_else(|| {
            serde::de::Error::custom(format!("Invalid QaComponentFlags bits: {}", bits))
        })
    }
}

/// Quality assurance level presets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QaLevel {
    /// Minimal checks for development
    Minimal,
    /// Standard checks for normal builds
    Standard,
    /// Strict checks for release builds
    Strict,
    /// Custom configuration
    Custom,
}

/// Quality assurance configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaConfig {
    /// QA level preset
    pub level: QaLevel,
    /// Component flags
    pub flags: QaComponentFlags,
    /// Maximum parallel jobs
    pub parallel_jobs: usize,
    /// Timeout for each check in seconds
    pub check_timeout: u64,
    /// Linter configurations by language
    pub linters: HashMap<String, LinterConfig>,
    /// Security scanner configurations
    pub scanners: HashMap<String, ScannerConfig>,
    /// Policy rules
    pub policy_rules: Vec<PolicyRule>,
    /// Paths to exclude from checks
    pub exclude_paths: Vec<PathBuf>,
    /// File patterns to exclude (glob patterns)
    pub exclude_patterns: Vec<String>,
    /// Report output format
    pub report_format: ReportFormat,
    /// Report output path (if not specified, returns in-memory)
    pub report_path: Option<PathBuf>,
}

/// Report output format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportFormat {
    /// Human-readable text
    Text,
    /// JSON format
    Json,
    /// SARIF (Static Analysis Results Interchange Format)
    Sarif,
    /// JUnit XML format
    JUnit,
}

impl Default for QaConfig {
    fn default() -> Self {
        Self::standard()
    }
}

impl QaConfig {
    /// Create minimal configuration for development
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            level: QaLevel::Minimal,
            flags: QaComponentFlags::minimal(),
            parallel_jobs: 4,
            check_timeout: 300, // 5 minutes
            linters: Self::minimal_linters(),
            scanners: HashMap::new(),
            policy_rules: Vec::new(),
            exclude_paths: vec![
                PathBuf::from("target"),
                PathBuf::from("node_modules"),
                PathBuf::from("vendor"),
                PathBuf::from(".git"),
            ],
            exclude_patterns: vec![
                "*.min.js".to_string(),
                "*.min.css".to_string(),
                "*_test.go".to_string(),
            ],
            report_format: ReportFormat::Text,
            report_path: None,
        }
    }

    /// Create standard configuration
    #[must_use]
    pub fn standard() -> Self {
        Self {
            level: QaLevel::Standard,
            flags: QaComponentFlags::default(),
            parallel_jobs: 8,
            check_timeout: 600, // 10 minutes
            linters: Self::standard_linters(),
            scanners: Self::standard_scanners(),
            policy_rules: Self::standard_policies(),
            exclude_paths: vec![
                PathBuf::from("target"),
                PathBuf::from("node_modules"),
                PathBuf::from("vendor"),
                PathBuf::from(".git"),
            ],
            exclude_patterns: vec![
                "*.min.js".to_string(),
                "*.min.css".to_string(),
                "*_test.go".to_string(),
            ],
            report_format: ReportFormat::Json,
            report_path: None,
        }
    }

    /// Create strict configuration for release builds
    #[must_use]
    pub fn strict() -> Self {
        Self {
            level: QaLevel::Strict,
            flags: QaComponentFlags::strict(),
            parallel_jobs: 16,
            check_timeout: 1200, // 20 minutes
            linters: Self::strict_linters(),
            scanners: Self::strict_scanners(),
            policy_rules: Self::strict_policies(),
            exclude_paths: vec![PathBuf::from(".git")],
            exclude_patterns: Vec::new(),
            report_format: ReportFormat::Sarif,
            report_path: None,
        }
    }

    fn minimal_linters() -> HashMap<String, LinterConfig> {
        let mut linters = HashMap::new();

        // Rust - clippy only
        linters.insert(
            "clippy".to_string(),
            LinterConfig {
                command: "cargo".to_string(),
                args: vec![
                    "clippy".to_string(),
                    "--".to_string(),
                    "-W".to_string(),
                    "clippy::all".to_string(),
                ],
                extensions: vec!["rs".to_string()],
                enabled: true,
                env: HashMap::new(),
            },
        );

        linters
    }

    fn standard_linters() -> HashMap<String, LinterConfig> {
        let mut linters = HashMap::new();

        // Rust
        linters.insert(
            "clippy".to_string(),
            LinterConfig {
                command: "cargo".to_string(),
                args: vec![
                    "clippy".to_string(),
                    "--all-targets".to_string(),
                    "--".to_string(),
                    "-W".to_string(),
                    "clippy::pedantic".to_string(),
                ],
                extensions: vec!["rs".to_string()],
                enabled: true,
                env: HashMap::new(),
            },
        );

        linters.insert(
            "rustfmt".to_string(),
            LinterConfig {
                command: "cargo".to_string(),
                args: vec!["fmt".to_string(), "--check".to_string()],
                extensions: vec!["rs".to_string()],
                enabled: true,
                env: HashMap::new(),
            },
        );

        // C/C++
        linters.insert(
            "clang-tidy".to_string(),
            LinterConfig {
                command: "clang-tidy".to_string(),
                args: vec![],
                extensions: vec![
                    "c".to_string(),
                    "cpp".to_string(),
                    "cc".to_string(),
                    "cxx".to_string(),
                ],
                enabled: true,
                env: HashMap::new(),
            },
        );

        // Python
        linters.insert(
            "ruff".to_string(),
            LinterConfig {
                command: "ruff".to_string(),
                args: vec!["check".to_string()],
                extensions: vec!["py".to_string()],
                enabled: true,
                env: HashMap::new(),
            },
        );

        // JavaScript/TypeScript
        linters.insert(
            "eslint".to_string(),
            LinterConfig {
                command: "eslint".to_string(),
                args: vec![],
                extensions: vec![
                    "js".to_string(),
                    "jsx".to_string(),
                    "ts".to_string(),
                    "tsx".to_string(),
                ],
                enabled: true,
                env: HashMap::new(),
            },
        );

        linters
    }

    fn strict_linters() -> HashMap<String, LinterConfig> {
        let mut linters = Self::standard_linters();

        // Add more strict checks
        if let Some(clippy) = linters.get_mut("clippy") {
            clippy.args = vec![
                "clippy".to_string(),
                "--all-targets".to_string(),
                "--all-features".to_string(),
                "--".to_string(),
                "-D".to_string(),
                "clippy::all".to_string(),
                "-D".to_string(),
                "clippy::pedantic".to_string(),
                "-D".to_string(),
                "clippy::nursery".to_string(),
            ];
        }

        // Add cppcheck for C/C++
        linters.insert(
            "cppcheck".to_string(),
            LinterConfig {
                command: "cppcheck".to_string(),
                args: vec![
                    "--enable=all".to_string(),
                    "--inconclusive".to_string(),
                    "--suppress=missingInclude".to_string(),
                ],
                extensions: vec![
                    "c".to_string(),
                    "cpp".to_string(),
                    "cc".to_string(),
                    "cxx".to_string(),
                ],
                enabled: true,
                env: HashMap::new(),
            },
        );

        // Add mypy for Python
        linters.insert(
            "mypy".to_string(),
            LinterConfig {
                command: "mypy".to_string(),
                args: vec!["--strict".to_string()],
                extensions: vec!["py".to_string()],
                enabled: true,
                env: HashMap::new(),
            },
        );

        linters
    }

    fn standard_scanners() -> HashMap<String, ScannerConfig> {
        let mut scanners = HashMap::new();

        // Rust security
        scanners.insert(
            "cargo-audit".to_string(),
            ScannerConfig {
                command: "cargo".to_string(),
                args: vec!["audit".to_string()],
                enabled: true,
                severity_threshold: QaSeverity::Warning,
                env: HashMap::new(),
            },
        );

        // General vulnerability scanner
        scanners.insert(
            "trivy".to_string(),
            ScannerConfig {
                command: "trivy".to_string(),
                args: vec![
                    "fs".to_string(),
                    "--severity".to_string(),
                    "MEDIUM,HIGH,CRITICAL".to_string(),
                ],
                enabled: true,
                severity_threshold: QaSeverity::Warning,
                env: HashMap::new(),
            },
        );

        scanners
    }

    fn strict_scanners() -> HashMap<String, ScannerConfig> {
        let mut scanners = Self::standard_scanners();

        // Python security
        scanners.insert(
            "bandit".to_string(),
            ScannerConfig {
                command: "bandit".to_string(),
                args: vec!["-r".to_string(), "-ll".to_string()],
                enabled: true,
                severity_threshold: QaSeverity::Info,
                env: HashMap::new(),
            },
        );

        // Node.js security
        scanners.insert(
            "npm-audit".to_string(),
            ScannerConfig {
                command: "npm".to_string(),
                args: vec!["audit".to_string(), "--json".to_string()],
                enabled: true,
                severity_threshold: QaSeverity::Info,
                env: HashMap::new(),
            },
        );

        // Update trivy to be more strict
        if let Some(trivy) = scanners.get_mut("trivy") {
            trivy.args = vec![
                "fs".to_string(),
                "--severity".to_string(),
                "LOW,MEDIUM,HIGH,CRITICAL".to_string(),
                "--exit-code".to_string(),
                "1".to_string(),
            ];
        }

        scanners
    }

    fn standard_policies() -> Vec<PolicyRule> {
        vec![
            PolicyRule {
                id: "license-compliance".to_string(),
                name: "License Compliance".to_string(),
                description: "Ensure all dependencies have compatible licenses".to_string(),
                severity: QaSeverity::Error,
                enabled: true,
                config: HashMap::new(),
            },
            PolicyRule {
                id: "binary-size-limit".to_string(),
                name: "Binary Size Limit".to_string(),
                description: "Ensure binaries don't exceed size limits".to_string(),
                severity: QaSeverity::Warning,
                enabled: true,
                config: {
                    let mut config = HashMap::new();
                    config.insert("max_size_mb".to_string(), serde_json::json!(100));
                    config
                },
            },
        ]
    }

    fn strict_policies() -> Vec<PolicyRule> {
        let mut policies = Self::standard_policies();

        policies.extend(vec![
            PolicyRule {
                id: "file-permissions".to_string(),
                name: "File Permission Check".to_string(),
                description: "Ensure files have appropriate permissions".to_string(),
                severity: QaSeverity::Error,
                enabled: true,
                config: HashMap::new(),
            },
            PolicyRule {
                id: "dependency-versions".to_string(),
                name: "Dependency Version Policy".to_string(),
                description: "Ensure dependencies meet version requirements".to_string(),
                severity: QaSeverity::Warning,
                enabled: true,
                config: HashMap::new(),
            },
            PolicyRule {
                id: "no-hardcoded-secrets".to_string(),
                name: "No Hardcoded Secrets".to_string(),
                description: "Ensure no secrets are hardcoded in source".to_string(),
                severity: QaSeverity::Critical,
                enabled: true,
                config: HashMap::new(),
            },
        ]);

        policies
    }

    /// Set report format
    #[must_use]
    pub fn with_report_format(mut self, format: ReportFormat) -> Self {
        self.report_format = format;
        self
    }

    /// Set report output path
    #[must_use]
    pub fn with_report_path(mut self, path: PathBuf) -> Self {
        self.report_path = Some(path);
        self
    }

    /// Enable or disable specific linter
    pub fn set_linter_enabled(&mut self, name: &str, enabled: bool) {
        if let Some(linter) = self.linters.get_mut(name) {
            linter.enabled = enabled;
        }
    }

    /// Enable or disable specific scanner
    pub fn set_scanner_enabled(&mut self, name: &str, enabled: bool) {
        if let Some(scanner) = self.scanners.get_mut(name) {
            scanner.enabled = enabled;
        }
    }

    /// Enable or disable specific policy rule
    pub fn set_policy_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(rule) = self.policy_rules.iter_mut().find(|r| r.id == id) {
            rule.enabled = enabled;
        }
    }

    /// Check if linters are enabled
    #[must_use]
    pub fn linters_enabled(&self) -> bool {
        self.flags.linters()
    }

    /// Check if scanners are enabled
    #[must_use]
    pub fn scanners_enabled(&self) -> bool {
        self.flags.scanners()
    }

    /// Check if policy validators are enabled
    #[must_use]
    pub fn policy_validators_enabled(&self) -> bool {
        self.flags.policy_validators()
    }

    /// Check if build should fail on warnings
    #[must_use]
    pub fn fail_on_warnings(&self) -> bool {
        self.flags.fail_on_warnings()
    }
}
