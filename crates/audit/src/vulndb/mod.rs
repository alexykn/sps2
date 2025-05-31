//! Vulnerability database management module

mod manager;
pub(crate) mod parser;
mod sources;

pub use manager::{DatabaseStatistics, VulnDbManager, VulnerabilityDatabase};
