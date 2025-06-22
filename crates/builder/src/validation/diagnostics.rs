//! Diagnostic reporting for post-validation issues

use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// A detailed diagnostic finding from validation
#[derive(Debug, Clone)]
pub struct ValidationFinding {
    /// The file where the issue was found
    pub file_path: PathBuf,
    /// The type of issue found
    pub issue_type: IssueType,
    /// Additional context about the finding
    pub context: HashMap<String, String>,
}

/// Types of validation issues
#[derive(Debug, Clone)]
pub enum IssueType {
    /// Hardcoded build path found
    HardcodedBuildPath { path: String, offset: Option<usize> },
    /// Hardcoded placeholder path found
    HardcodedPlaceholder { path: String, offset: Option<usize> },
    /// Bad RPATH in Mach-O binary
    BadRPath { rpath: String },
    /// Bad install name in Mach-O binary
    BadInstallName { install_name: String },
    /// Self-referencing install name in Mach-O binary
    SelfReferencingInstallName { install_name: String },
    /// Build path in static archive
    BuildPathInArchive {
        path: String,
        member: Option<String>,
    },
    /// Generic issue with custom message
    Custom { message: String },
}

impl IssueType {
    /// Get a human-readable description of the issue
    pub fn description(&self) -> String {
        match self {
            Self::HardcodedBuildPath { path, .. } => {
                format!("Contains hardcoded build path: {path}")
            }
            Self::HardcodedPlaceholder { path, .. } => {
                format!("Contains placeholder path: {path}")
            }
            Self::BadRPath { rpath } => {
                format!("Contains bad RPATH: {rpath}")
            }
            Self::BadInstallName { install_name } => {
                format!("Contains bad install name: {install_name}")
            }
            Self::SelfReferencingInstallName { install_name } => {
                format!("Contains self-referencing install name: {install_name}")
            }
            Self::BuildPathInArchive { path, member } => {
                if let Some(member) = member {
                    format!("Archive member '{member}' contains build path: {path}")
                } else {
                    format!("Archive contains build path: {path}")
                }
            }
            Self::Custom { message } => message.clone(),
        }
    }
}

/// Collector for validation findings
#[derive(Debug, Default)]
pub struct DiagnosticCollector {
    findings: Vec<ValidationFinding>,
}

impl DiagnosticCollector {
    /// Create a new diagnostic collector
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a finding to the collector
    pub fn add_finding(&mut self, finding: ValidationFinding) {
        self.findings.push(finding);
    }

    /// Add a hardcoded path finding
    pub fn add_hardcoded_path(
        &mut self,
        file_path: impl Into<PathBuf>,
        path: impl Into<String>,
        is_placeholder: bool,
    ) {
        let issue_type = if is_placeholder {
            IssueType::HardcodedPlaceholder {
                path: path.into(),
                offset: None,
            }
        } else {
            IssueType::HardcodedBuildPath {
                path: path.into(),
                offset: None,
            }
        };

        self.add_finding(ValidationFinding {
            file_path: file_path.into(),
            issue_type,
            context: HashMap::new(),
        });
    }

    /// Add a Mach-O issue
    pub fn add_macho_issue(&mut self, file_path: impl Into<PathBuf>, issue_type: IssueType) {
        self.add_finding(ValidationFinding {
            file_path: file_path.into(),
            issue_type,
            context: HashMap::new(),
        });
    }

    /// Check if there are any findings
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }

    /// Get the number of findings
    pub fn count(&self) -> usize {
        self.findings.len()
    }

    /// Get all findings
    pub fn findings(&self) -> &[ValidationFinding] {
        &self.findings
    }

    /// Take all findings, consuming the collector
    pub fn into_findings(self) -> Vec<ValidationFinding> {
        self.findings
    }

    /// Generate a summary by file
    pub fn summarize_by_file(&self) -> HashMap<&Path, Vec<&ValidationFinding>> {
        let mut summary: HashMap<&Path, Vec<&ValidationFinding>> = HashMap::new();
        for finding in &self.findings {
            summary
                .entry(finding.file_path.as_path())
                .or_default()
                .push(finding);
        }
        summary
    }

    /// Generate detailed diagnostic messages suitable for event emission
    pub fn generate_diagnostic_messages(&self) -> Vec<String> {
        let mut messages = Vec::new();

        // Group by file for better readability
        let by_file = self.summarize_by_file();

        for (file_path, findings) in by_file {
            let mut file_msg = format!("File: {}", file_path.display());
            for finding in findings {
                write!(file_msg, "\n  - {}", finding.issue_type.description()).unwrap();
            }
            messages.push(file_msg);
        }

        messages
    }

    /// Get all files that have hardcoded build paths or placeholders
    pub fn get_files_with_hardcoded_paths(&self) -> HashMap<&Path, Vec<&ValidationFinding>> {
        let mut result: HashMap<&Path, Vec<&ValidationFinding>> = HashMap::new();

        for finding in &self.findings {
            match &finding.issue_type {
                IssueType::HardcodedBuildPath { .. } | IssueType::HardcodedPlaceholder { .. } => {
                    result
                        .entry(finding.file_path.as_path())
                        .or_default()
                        .push(finding);
                }
                _ => {}
            }
        }

        result
    }

    /// Get all files that have Mach-O issues (bad RPATHs, install names, etc.)
    pub fn get_files_with_macho_issues(&self) -> HashMap<&Path, Vec<&ValidationFinding>> {
        let mut result: HashMap<&Path, Vec<&ValidationFinding>> = HashMap::new();

        for finding in &self.findings {
            match &finding.issue_type {
                IssueType::BadRPath { .. }
                | IssueType::BadInstallName { .. }
                | IssueType::SelfReferencingInstallName { .. } => {
                    result
                        .entry(finding.file_path.as_path())
                        .or_default()
                        .push(finding);
                }
                _ => {}
            }
        }

        result
    }

    /// Get all findings for a specific file
    pub fn get_findings_for_file(&self, path: &Path) -> Vec<&ValidationFinding> {
        self.findings
            .iter()
            .filter(|f| f.file_path == path)
            .collect()
    }

    /// Filter findings by issue type
    pub fn filter_by_issue_type<F>(&self, predicate: F) -> Vec<&ValidationFinding>
    where
        F: Fn(&IssueType) -> bool,
    {
        self.findings
            .iter()
            .filter(|f| predicate(&f.issue_type))
            .collect()
    }
}
