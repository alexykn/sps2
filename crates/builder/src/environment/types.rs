//! Types and result structures for build environment

use std::fmt;

use serde::de::{self, IgnoredAny, MapAccess, Unexpected, Visitor};
use std::path::PathBuf;

/// Result of executing a build command
#[derive(Debug)]
pub struct BuildCommandResult {
    /// Whether the command succeeded
    pub success: bool,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
}

/// Result of the build process
#[derive(Debug, Clone)]
pub struct BuildResult {
    /// Path to the generated package file
    pub package_path: PathBuf,
    /// SBOM files generated
    pub sbom_files: Vec<PathBuf>,
    /// Build log
    pub build_log: String,
    /// Whether the recipe requested the package be installed after building
    pub install_requested: bool,
}

impl BuildResult {
    /// Create new build result
    #[must_use]
    pub fn new(package_path: PathBuf) -> Self {
        Self {
            package_path,
            sbom_files: Vec::new(),
            build_log: String::new(),
            install_requested: false,
        }
    }

    /// Add SBOM file
    pub fn add_sbom_file(&mut self, path: PathBuf) {
        self.sbom_files.push(path);
    }

    /// Set build log
    pub fn set_build_log(&mut self, log: String) {
        self.build_log = log;
    }

    /// Set install requested flag
    #[must_use]
    pub fn with_install_requested(mut self, install_requested: bool) -> Self {
        self.install_requested = install_requested;
        self
    }
}

/// Build isolation level
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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

impl<'de> serde::Deserialize<'de> for IsolationLevel {
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
                    // Ensure no additional entries are present
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
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Default => write!(f, "default"),
            Self::Enhanced => write!(f, "enhanced"),
            Self::Hermetic => write!(f, "hermetic"),
        }
    }
}
