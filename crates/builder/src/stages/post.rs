//! Post-processing stage types and operations

use serde::{Deserialize, Serialize};
use sps2_types::RpathStyle;

/// Post-processing operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PostStep {
    /// Patch rpaths in binaries
    PatchRpaths {
        style: RpathStyle,
        paths: Vec<String>,
    },

    /// Fix executable permissions
    FixPermissions { paths: Vec<String> },

    /// Run arbitrary command in post stage
    Command { program: String, args: Vec<String> },
}

// Note: ParsedPost is recipe::model::Post
