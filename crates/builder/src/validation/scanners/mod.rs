//! Registry of all scanner (validator) modules.

pub mod archive;
pub mod hardcoded;
pub mod macho;

// Re-export the concrete types for convenient access elsewhere.
pub use archive::ArchiveScanner;
pub use hardcoded::HardcodedScanner;
pub use macho::MachOScanner;
