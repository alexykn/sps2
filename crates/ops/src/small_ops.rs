//! Small operations implemented in the ops crate
//!
//! This module serves as a public API facade that re-exports operations
//! from specialized modules. All function signatures are preserved for
//! backward compatibility.

// Import all the modularized operations
use crate::health;
use crate::maintenance;
use crate::query;
use crate::repository;
use crate::self_update as self_update_module;

// Re-export all public functions to maintain API compatibility
pub use health::check_health;
pub use maintenance::{cleanup, history, rollback};
pub use query::{list_packages, package_info, search_packages};
pub use repository::{add_repo, list_repos, remove_repo, reposync};
pub use self_update_module::self_update;
