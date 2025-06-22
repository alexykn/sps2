//! Recipe parsing and execution module

pub mod executor;
pub mod model;
pub mod parser;

// Re-export commonly used items
pub use executor::execute_recipe;
