//! Verification logic for packages and files

pub mod content;
pub mod package;
pub mod scope;

// Re-export key functions
pub use content::verify_file_content;
pub use package::verify_package;
pub use scope::{count_total_files, get_packages_for_scope};
