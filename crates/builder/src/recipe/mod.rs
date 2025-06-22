//! Recipe parsing and execution module

pub mod executor;
pub mod model;
pub mod parser;

// Re-export commonly used items
pub use executor::{execute_build_step, execute_build_steps_list, execute_recipe};
