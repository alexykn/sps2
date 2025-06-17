//! Small helpers for collecting diagnostics from validators / patchers.

use std::fmt::Write;

#[derive(Default, Debug)]
pub struct Report {
    pub changed_files: Vec<std::path::PathBuf>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
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
}
