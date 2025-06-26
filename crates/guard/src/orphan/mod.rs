//! Orphaned file detection and handling

pub mod categorization;
pub mod detection;

// Re-export key functions
pub use categorization::categorize_orphaned_file;
pub use detection::{find_orphaned_files, find_orphaned_files_scoped, get_directories_to_check};
