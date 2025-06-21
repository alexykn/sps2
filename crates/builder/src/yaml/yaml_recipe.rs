//! YAML recipe format for sps2
//!
//! This module provides a declarative YAML-based recipe format that replaces
//! the Starlark-based system with proper staged execution.

use crate::environment::IsolationLevel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Complete YAML recipe structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlRecipe {
    /// Package metadata (required)
    pub metadata: Metadata,

    /// Dynamic facts/variables (optional)
    #[serde(default)]
    pub facts: HashMap<String, String>,

    /// Environment setup stage (optional)
    #[serde(default)]
    pub environment: Environment,

    /// Source acquisition stage (required)
    pub source: Source,

    /// Build stage (required)
    pub build: Build,

    /// Post-processing stage (optional)
    #[serde(default)]
    pub post: Post,

    /// Installation behavior (optional)
    #[serde(default)]
    pub install: Install,
}

/// Package metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: String,

    #[serde(default)]
    pub homepage: Option<String>,

    #[serde(default)]
    pub dependencies: Dependencies,
}

/// Dependencies specification
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dependencies {
    #[serde(default)]
    pub runtime: Vec<String>,

    #[serde(default)]
    pub build: Vec<String>,
}

/// Environment setup stage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    /// Isolation level: none (0), standard (1), enhanced (2), hermetic (3)
    #[serde(default = "default_isolation")]
    pub isolation: IsolationLevel,

    /// Apply optimized compiler flags
    #[serde(default)]
    pub defaults: bool,

    /// Allow network during build
    #[serde(default)]
    pub network: bool,

    /// Environment variables
    #[serde(default)]
    pub variables: HashMap<String, String>,
}

fn default_isolation() -> IsolationLevel {
    IsolationLevel::Default
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            isolation: default_isolation(),
            defaults: false,
            network: false,
            variables: HashMap::new(),
        }
    }
}

/// Source acquisition stage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Git source
    #[serde(flatten)]
    pub method: SourceMethod,

    /// Patches to apply after extraction
    #[serde(default)]
    pub patches: Vec<String>,
}

/// Source acquisition methods
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SourceMethod {
    Git { git: GitSource },
    Fetch { fetch: FetchSource },
    Local { local: LocalSource },
}

/// Git source specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitSource {
    pub url: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
}

/// Fetch source specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchSource {
    pub url: String,
    #[serde(default)]
    pub checksum: Option<Checksum>,
}

/// Checksum specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checksum {
    #[serde(flatten)]
    pub algorithm: ChecksumAlgorithm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChecksumAlgorithm {
    Blake3 { blake3: String },
    Sha256 { sha256: String },
    Md5 { md5: String },
}

/// Local source specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSource {
    pub path: String,
}

/// Build stage
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Build {
    /// Simple build system invocation
    System {
        system: BuildSystem,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Complex build with custom steps
    Steps { steps: Vec<BuildStep> },
}

/// Supported build systems
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildSystem {
    Autotools,
    Cmake,
    Meson,
    Cargo,
    Make,
    Go,
    Python,
    Nodejs,
}

/// Individual build step
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BuildStep {
    Command { command: String },
    Make { make: Vec<String> },
    Configure { configure: Vec<String> },
    Cmake { cmake: Vec<String> },
    Meson { meson: Vec<String> },
    Cargo { cargo: Vec<String> },
    Go { go: Vec<String> },
    Python { python: Vec<String> },
    Nodejs { nodejs: Vec<String> },
}

/// Post-processing stage
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Post {
    /// Fix rpaths (convert @rpath to absolute paths)
    #[serde(default)]
    pub patch_rpaths: PostOption,

    /// Fix executable permissions
    #[serde(default)]
    pub fix_permissions: PostOption,

    /// Custom post-processing commands
    #[serde(default)]
    pub commands: Vec<String>,
}

/// Post-processing option (bool or list of paths)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PostOption {
    Enabled(bool),
    Paths(Vec<String>),
}

impl Default for PostOption {
    fn default() -> Self {
        PostOption::Enabled(false)
    }
}

/// Installation behavior
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Install {
    /// Auto-install after building
    #[serde(default)]
    pub auto: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_recipe() {
        let yaml = r"
metadata:
  name: zlib
  version: 1.3.1
  description: General-purpose lossless data compression library
  license: Zlib

source:
  fetch:
    url: https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz

build:
  system: cmake
  args:
    - -DCMAKE_BUILD_TYPE=Release
";
        let recipe: YamlRecipe = serde_yml::from_str(yaml).unwrap();
        assert_eq!(recipe.metadata.name, "zlib");
        assert_eq!(recipe.metadata.version, "1.3.1");
    }

    #[test]
    fn test_parse_complex_recipe() {
        let yaml = r#"
metadata:
  name: gcc
  version: 15.1.0
  description: GNU Compiler Collection
  license: GPL-3.0-or-later
  dependencies:
    build:
      - gmp
      - mpfr

facts:
  build_triple: aarch64-apple-darwin24

environment:
  isolation: default
  defaults: true
  variables:
    LDFLAGS: "-L${PREFIX}/lib"

source:
  local:
    path: ./src
  patches:
    - gcc-darwin.patch

build:
  steps:
    - command: mkdir -p build
    - command: cd build && ../configure --build=${build_triple}

post:
  fix_permissions: true
"#;
        let recipe: YamlRecipe = serde_yml::from_str(yaml).unwrap();
        assert_eq!(recipe.metadata.name, "gcc");
        assert_eq!(
            recipe.facts.get("build_triple").unwrap(),
            "aarch64-apple-darwin24"
        );
    }
}
