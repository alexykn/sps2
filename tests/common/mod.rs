//! Common test utilities and infrastructure for download testing
//!
//! This module provides:
//! - Mock HTTP server infrastructure
//! - Test package generation utilities  
//! - Network simulation helpers
//! - Test event collection utilities

pub mod mock_server;
pub mod network_simulation;
pub mod package_generator;
pub mod repo_simulation;
pub mod test_helpers;

// Re-export key types that are commonly used by download_tests.rs (which uses common::*)
pub use mock_server::{ConfigurableMockServer, MockServerConfig, ThrottledHttpServer};
pub use package_generator::{
    ContentPattern, MalformationType, TestPackageConfig, TestPackageGenerator,
};
pub use test_helpers::{
    EventVerifier, FileValidator, PerformanceBenchmark, TestDataGenerator, TestDataPattern,
    TestEnvironment,
};
