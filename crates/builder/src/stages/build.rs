//! Build stage types and operations

use serde::{Deserialize, Serialize};

/// Build commands that can be executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BuildCommand {
    /// Run configure script
    Configure { args: Vec<String> },

    /// Run make
    Make { args: Vec<String> },

    /// Run autotools build
    Autotools { args: Vec<String> },

    /// Run `CMake` build
    Cmake { args: Vec<String> },

    /// Run Meson build
    Meson { args: Vec<String> },

    /// Run Cargo build
    Cargo { args: Vec<String> },

    /// Run Go build
    Go { args: Vec<String> },

    /// Run Python build
    Python { args: Vec<String> },

    /// Run Node.js build
    NodeJs { args: Vec<String> },

    /// Run arbitrary command
    Command { program: String, args: Vec<String> },
}

// Note: ParsedBuild is recipe::model::Build
// Note: ParsedStep is recipe::model::ParsedStep
