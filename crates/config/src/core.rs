//! Core configuration types and utilities shared across all crates

use super::repository::Repositories;
use serde::{Deserialize, Serialize};
use sps2_types::{ColorChoice, OutputFormat};
use std::path::PathBuf;

/// General application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_output_format")]
    pub default_output: OutputFormat,
    #[serde(default = "default_color_choice")]
    pub color: ColorChoice,
    #[serde(default = "default_parallel_downloads")]
    pub parallel_downloads: usize,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_output: OutputFormat::Tty,
            color: ColorChoice::Auto,
            parallel_downloads: 4,
        }
    }
}

/// Security configuration shared across crates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_verify_signatures")]
    pub verify_signatures: bool,
    #[serde(default = "default_allow_unsigned")]
    pub allow_unsigned: bool,
    #[serde(default = "default_index_max_age_days")]
    pub index_max_age_days: u32,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            verify_signatures: true,
            allow_unsigned: false,
            index_max_age_days: 7,
        }
    }
}

/// State management configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateConfig {
    #[serde(default = "default_retention_count")]
    pub retention_count: usize,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

impl Default for StateConfig {
    fn default() -> Self {
        Self {
            retention_count: 10, // Keep last 10 states
            retention_days: 30,  // Or 30 days, whichever is less
        }
    }
}

/// Path configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PathConfig {
    pub store_path: Option<PathBuf>,
    pub state_path: Option<PathBuf>,
    pub build_path: Option<PathBuf>,
}

/// Repository configuration group
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepositoryGroupConfig {
    #[serde(default)]
    pub repositories: Repositories,
}

/// Network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64, // seconds
    #[serde(default = "default_retries")]
    pub retries: u32,
    #[serde(default = "default_retry_delay")]
    pub retry_delay: u64, // seconds
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            timeout: 300, // 5 minutes
            retries: 3,
            retry_delay: 1, // 1 second
        }
    }
}

// Default value functions for serde
fn default_output_format() -> OutputFormat {
    OutputFormat::Tty
}

fn default_color_choice() -> ColorChoice {
    ColorChoice::Auto
}

fn default_parallel_downloads() -> usize {
    4
}

fn default_verify_signatures() -> bool {
    true
}

fn default_allow_unsigned() -> bool {
    false
}

fn default_index_max_age_days() -> u32 {
    7
}

fn default_retention_count() -> usize {
    10
}

fn default_retention_days() -> u32 {
    30
}

fn default_timeout() -> u64 {
    300 // 5 minutes
}

fn default_retries() -> u32 {
    3
}

fn default_retry_delay() -> u64 {
    1 // 1 second
}
