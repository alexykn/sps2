//! Vulnerability database management module

// mod cache;
mod database;
mod manager;
pub(crate) mod parser;
mod schema;
mod sources;
mod statistics;
mod updater;

// Re-export main types for external use
pub use database::VulnerabilityDatabase;
pub use manager::VulnDbManager;
pub use statistics::DatabaseStatistics;

// Internal cache types for future optimization

// use cache::{CacheStatistics, VulnerabilityCache};
