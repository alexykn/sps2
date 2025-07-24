//! macOS-specific platform implementation

pub mod binary;
pub mod filesystem;
pub mod process;

/// macOS platform implementation
pub struct MacOSPlatform;

impl MacOSPlatform {
    /// Create a new macOS platform instance
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> crate::core::Platform {
        use binary::MacOSBinaryOperations;
        use filesystem::MacOSFilesystemOperations;
        use process::MacOSProcessOperations;
        
        crate::core::Platform::new(
            Box::new(MacOSBinaryOperations::new()),
            Box::new(MacOSFilesystemOperations::new()),
            Box::new(MacOSProcessOperations::new()),
        )
    }
}