//! Source stage types and operations

use serde::{Deserialize, Serialize};

/// Source operations that can be executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceStep {
    /// Clean the source directory
    Cleanup,

    /// Fetch file from URL
    Fetch { url: String },

    /// Fetch with MD5 verification
    FetchMd5 { url: String, md5: String },

    /// Fetch with SHA256 verification
    FetchSha256 { url: String, sha256: String },

    /// Fetch with BLAKE3 verification
    FetchBlake3 { url: String, blake3: String },

    /// Extract downloaded archives
    Extract,

    /// Clone from git
    Git { url: String, ref_: String },

    /// Copy local files
    Copy { src_path: Option<String> },

    /// Apply a patch
    ApplyPatch { path: String },
}

// Note: ParsedSource is recipe::model::Source
