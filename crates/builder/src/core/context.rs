//! Build context for package building

use sps2_events::{EventEmitter, EventSender};
use sps2_types::Version;
use std::path::PathBuf;

/// Build context for package building
#[derive(Clone, Debug)]
pub struct BuildContext {
    /// Package name
    pub name: String,
    /// Package version
    pub version: Version,
    /// Revision number
    pub revision: u32,
    /// Target architecture
    pub arch: String,
    /// Recipe file path
    pub recipe_path: PathBuf,
    /// Output directory for .sp files
    pub output_dir: PathBuf,
    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
    /// Path to the generated .sp package (set after package creation)
    pub package_path: Option<PathBuf>,
}

impl EventEmitter for BuildContext {
    fn event_sender(&self) -> Option<&EventSender> {
        self.event_sender.as_ref()
    }
}

impl BuildContext {
    /// Create new build context
    #[must_use]
    pub fn new(name: String, version: Version, recipe_path: PathBuf, output_dir: PathBuf) -> Self {
        Self {
            name,
            version,
            revision: 1,
            arch: "arm64".to_string(),
            recipe_path,
            output_dir,
            event_sender: None,
            package_path: None,
        }
    }

    /// Set revision number
    #[must_use]
    pub fn with_revision(mut self, revision: u32) -> Self {
        self.revision = revision;
        self
    }

    /// Set architecture
    #[must_use]
    pub fn with_arch(mut self, arch: String) -> Self {
        self.arch = arch;
        self
    }

    /// Set event sender
    #[must_use]
    pub fn with_event_sender(mut self, event_sender: EventSender) -> Self {
        self.event_sender = Some(event_sender);
        self
    }

    /// Get package filename
    #[must_use]
    pub fn package_filename(&self) -> String {
        format!(
            "{}-{}-{}.{}.sp",
            self.name, self.version, self.revision, self.arch
        )
    }

    /// Get full output path
    #[must_use]
    pub fn output_path(&self) -> PathBuf {
        self.output_dir.join(self.package_filename())
    }
}
