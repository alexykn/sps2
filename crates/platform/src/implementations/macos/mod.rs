//! macOS-specific platform implementation

use crate::binary::BinaryOperations;
use crate::filesystem::FilesystemOperations;
use crate::process::ProcessOperations;
use crate::core::Platform;

pub mod binary;
pub mod filesystem;
pub mod process;

use binary::MacOSBinaryOperations;
use filesystem::MacOSFilesystemOperations;
use process::MacOSProcessOperations;

/// macOS platform implementation
pub struct MacOSPlatform;

impl MacOSPlatform {
    /// Create a new macOS platform instance
    pub fn new() -> Platform {
        Platform::new(
            Box::new(MacOSBinaryOperations::new()),
            Box::new(MacOSFilesystemOperations::new()),
            Box::new(MacOSProcessOperations::new()),
        )
    }
}