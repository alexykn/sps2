//! Small helpers for collecting diagnostics from validators / patchers.

use std::fmt::Write;

use crate::post_validation::diagnostics::DiagnosticCollector;

#[derive(Default, Debug)]
pub struct Report {
    pub changed_files: Vec<std::path::PathBuf>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    /// Findings from validators that can be passed to patchers
    pub findings: Option<DiagnosticCollector>,
}
impl Report {
    pub fn ok() -> Self {
        Self::default()
    }
    pub fn is_fatal(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Add another report’s data into `self`.
    pub fn absorb(&mut self, other: Report) {
        self.changed_files.extend(other.changed_files);
        self.warnings.extend(other.warnings);
        self.errors.extend(other.errors);

        // Merge findings
        if let Some(other_findings) = other.findings {
            if let Some(ref mut our_findings) = self.findings {
                // Merge other findings into ours
                for finding in other_findings.into_findings() {
                    our_findings.add_finding(finding);
                }
            } else {
                // We don't have findings yet, take theirs
                self.findings = Some(other_findings);
            }
        }
    }

    pub fn render(&self, title: &str) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "{title}:");
        for e in &self.errors {
            let _ = writeln!(s, "  {e}");
        }
        for w in &self.warnings {
            let _ = writeln!(s, "  (warning) {w}");
        }
        s
    }
}

/// Convenience wrap that merges many [`Report`]s.
#[derive(Default)]
pub struct MergedReport(Report);
impl MergedReport {
    pub fn absorb(&mut self, r: Report) {
        self.0.absorb(r);
    }
    pub fn is_fatal(&self) -> bool {
        self.0.is_fatal()
    }
    pub fn render(&self, title: &str) -> String {
        self.0.render(title)
    }
    /// Get the collected findings
    pub fn findings(&self) -> Option<&DiagnosticCollector> {
        self.0.findings.as_ref()
    }
    /// Take the collected findings
    pub fn take_findings(&mut self) -> Option<DiagnosticCollector> {
        self.0.findings.take()
    }
}
