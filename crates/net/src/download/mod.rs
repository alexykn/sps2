//! Production-ready streaming download infrastructure for .sp files
//!
//! This module provides high-performance, resumable downloads with concurrent
//! signature verification and comprehensive error handling.

mod config;
mod core;
mod resume;
mod retry;
mod stream;
mod validation;

// Re-export public types and structs
pub use config::{
    DownloadProgress, DownloadResult, PackageDownloadConfig, PackageDownloadRequest,
    PackageDownloadResult, RetryConfig,
};
pub use core::PackageDownloader;
