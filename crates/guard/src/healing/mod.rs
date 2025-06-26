//! Healing functionality for state discrepancies

pub mod files;
pub mod orphans;
pub mod venv;

// Re-export key functions
pub use files::{heal_corrupted_file, is_user_modified_file, restore_missing_file};
pub use orphans::{
    backup_and_remove_orphaned_file, determine_orphaned_file_action, handle_orphaned_file,
    remove_orphaned_file,
};
pub use venv::heal_missing_venv;
