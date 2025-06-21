//! Recipe data structures

use serde::{Deserialize, Serialize};
use sps2_types::RpathStyle;

/// Recipe metadata collected from `metadata()` function
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecipeMetadata {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub runtime_deps: Vec<String>,
    pub build_deps: Vec<String>,
}

/// A build step from the `build()` function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BuildStep {
    Fetch {
        url: String,
    },
    FetchMd5 {
        url: String,
        md5: String,
    },
    FetchSha256 {
        url: String,
        sha256: String,
    },
    FetchBlake3 {
        url: String,
        blake3: String,
    },
    Extract,
    Git {
        url: String,
        ref_: String,
    },
    ApplyPatch {
        path: String,
    },
    AllowNetwork {
        enabled: bool,
    },
    Configure {
        args: Vec<String>,
    },
    Make {
        args: Vec<String>,
    },
    Autotools {
        args: Vec<String>,
    },
    Cmake {
        args: Vec<String>,
    },
    Meson {
        args: Vec<String>,
    },
    Cargo {
        args: Vec<String>,
    },
    Go {
        args: Vec<String>,
    },
    Python {
        args: Vec<String>,
    },
    NodeJs {
        args: Vec<String>,
    },
    Command {
        program: String,
        args: Vec<String>,
    },
    SetEnv {
        key: String,
        value: String,
    },
    WithDefaults,
    Install,
    // Cleanup staging directory
    Cleanup,
    // Copy source files
    Copy {
        src_path: Option<String>,
    },
    // Apply rpath patching to binaries and libraries
    PatchRpaths {
        style: RpathStyle,
        paths: Vec<String>,
    },
    // Fix executable permissions on binaries
    FixPermissions {
        paths: Vec<String>,
    },
    // Set build isolation level
    SetIsolation {
        level: u8,
    },
}
