#![deny(clippy::pedantic, unsafe_code)]

//! Resource management utilities for sps2
//!
//! This crate provides centralized resource management for coordinating
//! concurrent operations across the sps2 package manager. It includes
//! semaphore management, resource limits, and memory tracking.

pub mod limits;
pub mod manager;
pub mod semaphore;

pub use limits::{IntoResourceLimits, ResourceAvailability, ResourceLimits};
pub use manager::ResourceManager;
pub use semaphore::{acquire_semaphore_permit, create_semaphore, try_acquire_semaphore_permit};
