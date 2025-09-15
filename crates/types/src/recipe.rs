//! YAML recipe format for sps2
//!
//! This module provides a declarative YAML-based recipe format that replaces
//! the Starlark-based system with proper staged execution.

use serde::de::{self, IgnoredAny, MapAccess, Unexpected, Visitor};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt};

/// Build isolation level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

impl<'de> Deserialize<'de> for IsolationLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct IsolationLevelVisitor;

        impl<'de> Visitor<'de> for IsolationLevelVisitor {
            type Value = IsolationLevel;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an isolation level (none, default, enhanced, hermetic, or 0-3)")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                parse_from_str(value)
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                parse_from_str(&value)
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value > u64::from(u8::MAX) {
                    return Err(de::Error::invalid_value(
                        Unexpected::Unsigned(value),
                        &"number between 0 and 3",
                    ));
                }
                let byte = u8::try_from(value).map_err(|_| {
                    de::Error::invalid_value(Unexpected::Unsigned(value), &"number between 0 and 3")
                })?;
                parse_from_u8(byte)
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(de::Error::invalid_value(
                        Unexpected::Signed(value),
                        &"number between 0 and 3",
                    ));
                }
                let unsigned = u64::try_from(value).map_err(|_| {
                    de::Error::invalid_value(Unexpected::Signed(value), &"number between 0 and 3")
                })?;
                self.visit_u64(unsigned)
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                if let Some((key, _value)) = map.next_entry::<String, IgnoredAny>()? {
                    if map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {
                        return Err(de::Error::custom(
                            "isolation level map must contain a single entry",
                        ));
                    }
                    parse_from_str(&key)
                } else {
                    Err(de::Error::custom(
                        "expected isolation level map with a single entry",
                    ))
                }
            }
        }

        deserializer.deserialize_any(IsolationLevelVisitor)
    }
}

fn parse_from_str<E>(value: &str) -> Result<IsolationLevel, E>
where
    E: de::Error,
{
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "none" => Ok(IsolationLevel::None),
        "default" => Ok(IsolationLevel::Default),
        "enhanced" => Ok(IsolationLevel::Enhanced),
        "hermetic" => Ok(IsolationLevel::Hermetic),
        other => Err(de::Error::unknown_variant(
            other,
            &["none", "default", "enhanced", "hermetic"],
        )),
    }
}

fn parse_from_u8<E>(value: u8) -> Result<IsolationLevel, E>
where
    E: de::Error,
{
    IsolationLevel::from_u8(value).ok_or_else(|| {
        de::Error::invalid_value(
            Unexpected::Unsigned(u64::from(value)),
            &"number between 0 and 3",
        )
    })
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
    /// Source method (single source for backward compatibility)
    #[serde(flatten)]
    pub method: Option<SourceMethod>,

    /// Multiple sources (new multi-source support)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<NamedSource>,

    /// Patches to apply after extraction
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patches: Vec<String>,
}

/// Named source with optional extract location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedSource {
    /// Source method
    #[serde(flatten)]
    pub method: SourceMethod,

    /// Where to extract relative to build directory (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_to: Option<String>,
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
    /// Where to extract relative to build directory (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_to: Option<String>,
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

    /// QA pipeline override (auto, rust, c, go, python, skip)
    #[serde(default, skip_serializing_if = "crate::QaPipelineOverride::is_default")]
    pub qa_pipeline: crate::QaPipelineOverride,

    /// Custom post-processing commands
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

impl Post {
    /// Check if post-processing is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patch_rpaths.is_none()
            && self.fix_permissions.is_none()
            && self.qa_pipeline == crate::QaPipelineOverride::Auto
            && self.commands.is_empty()
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
