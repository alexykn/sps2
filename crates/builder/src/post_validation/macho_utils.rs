//! Shared utilities for working with Mach-O files
//! Used by both scanners and patchers to ensure consistent detection

use object::FileKind;
use std::path::Path;

/// Check if a file is a Mach-O binary by parsing its header
/// Uses the exact same logic as the MachO scanner
pub fn is_macho_file(path: &Path) -> bool {
    if let Ok(data) = std::fs::read(path) {
        FileKind::parse(&*data).is_ok()
    } else {
        false
    }
}
