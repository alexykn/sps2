use serde::{Deserialize, Serialize};
use sps2_types::Version;
/// Installation domain events consumed by the CLI and guard rails
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InstallEvent {
    /// Installation operation started for a package
    Started { package: String, version: Version },

    /// Installation completed successfully
    Completed {
        package: String,
        version: Version,
        files_installed: usize,
    },

    /// Installation failed
    Failed {
        package: String,
        version: Version,
        failure: super::FailureContext,
    },
}
