//! Modularized integration tests for sps2 package manager
//!
//! These tests exercise the full system end-to-end using test fixtures.
//! Tests are organized by functional area for better maintainability.

mod integration;

// Re-export all test modules
pub use integration::*;