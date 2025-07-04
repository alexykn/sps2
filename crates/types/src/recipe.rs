//! YAML recipe format for sps2
//!
//! This module provides a declarative YAML-based recipe format that replaces
//! the Starlark-based system with proper staged execution.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Build isolation level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IsolationLevel {
    /// No isolation - uses host environment as-is (shows warning)
    None = 0,
    /// Default isolation - clean environment, controlled paths (default)
    Default = 1,
    /// Enhanced isolation - default + private HOME/TMPDIR
    Enhanced = 2,
    /// Hermetic isolation - full whitelist approach, network blocking
    Hermetic = 3,
}

impl Default for IsolationLevel {
    fn default() -> Self {
        Self::Default
    }
}

impl IsolationLevel {
    /// Convert from u8
    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Default),
            2 => Some(Self::Enhanced),
            3 => Some(Self::Hermetic),
            _ => None,
        }
    }

    /// Convert to u8
    #[must_use]
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Check if this is the default isolation level
    #[must_use]
    pub fn is_default_value(self) -> bool {
        self == Self::Default
    }
}

impl std::fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Default => write!(f, "default"),
            Self::Enhanced => write!(f, "enhanced"),
            Self::Hermetic => write!(f, "hermetic"),
        }
    }
}

/// Complete YAML recipe structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlRecipe {
    /// Package metadata (required)
    pub metadata: Metadata,

    /// Dynamic facts/variables (optional)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub facts: HashMap<String, String>,

    /// Environment setup stage (optional)
    #[serde(default, skip_serializing_if = "Environment::is_default")]
    pub environment: Environment,

    /// Source acquisition stage (required)
    pub source: Source,

    /// Build stage (required)
    pub build: Build,

    /// Post-processing stage (optional)
    #[serde(default, skip_serializing_if = "Post::is_empty")]
    pub post: Post,

    /// Installation behavior (optional)
    #[serde(default, skip_serializing_if = "Install::is_default")]
    pub install: Install,
}

/// Package metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,

    #[serde(default, skip_serializing_if = "Dependencies::is_empty")]
    pub dependencies: Dependencies,
}

/// Dependencies specification
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dependencies {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build: Vec<String>,
}

impl Dependencies {
    /// Check if dependencies are empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runtime.is_empty() && self.build.is_empty()
    }
}

/// Environment setup stage
#[derive(Debug, Clone, Deserialize)]
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

impl Serialize for Environment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;

        if self.isolation != IsolationLevel::default() {
            map.serialize_entry("isolation", &self.isolation)?;
        }
        if self.defaults {
            map.serialize_entry("defaults", &self.defaults)?;
        }
        if self.network {
            map.serialize_entry("network", &self.network)?;
        }
        if !self.variables.is_empty() {
            map.serialize_entry("variables", &self.variables)?;
        }

        map.end()
    }
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

impl Environment {
    /// Check if environment is default
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.isolation == IsolationLevel::default()
            && !self.defaults
            && !self.network
            && self.variables.is_empty()
    }
}

/// Source acquisition stage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Source method
    #[serde(flatten)]
    pub method: SourceMethod,

    /// Patches to apply after extraction
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    Steps { steps: Vec<ParsedStep> },
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

/// Parsed build step from YAML recipe
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParsedStep {
    // Simple command (splits by whitespace, no shell features)
    Command { command: String },
    // Shell command (passed to sh -c, supports pipes/redirects/etc)
    Shell { shell: String },
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
    /// `RPath` patching behavior
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_rpaths: Option<String>,

    /// Fix executable permissions
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_permissions: Option<PostOption>,

    /// Custom post-processing commands
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

impl Post {
    /// Check if post-processing is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patch_rpaths.is_none() && self.fix_permissions.is_none() && self.commands.is_empty()
    }
}

/// Post-processing option (true/false or list of paths)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PostOption {
    Boolean(bool),
    Paths(Vec<String>),
}

/// Installation behavior
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Install {
    /// Automatically install after building
    #[serde(default)]
    pub auto: bool,
}

impl Serialize for Install {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;

        if self.auto {
            map.serialize_entry("auto", &self.auto)?;
        }

        map.end()
    }
}

impl Install {
    /// Check if install is default
    #[must_use]
    pub fn is_default(&self) -> bool {
        !self.auto
    }
}
