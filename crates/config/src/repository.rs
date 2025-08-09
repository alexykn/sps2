use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryConfig {
    pub url: String,
    #[serde(default = "default_priority")]
    pub priority: u32,
    #[serde(default = "default_algorithm")]
    pub algorithm: String, // "minisign" | "openpgp" (future)
    #[serde(default)]
    pub key_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Repositories {
    #[serde(default)]
    pub fast: Option<RepositoryConfig>,
    #[serde(default)]
    pub slow: Option<RepositoryConfig>,
    #[serde(default)]
    pub stable: Option<RepositoryConfig>,
    #[serde(default)]
    pub extras: std::collections::HashMap<String, RepositoryConfig>,
}

fn default_priority() -> u32 {
    1
}
fn default_algorithm() -> String {
    "minisign".to_string()
}
