use serde::{Deserialize, Serialize};
use sps2_types::Version;

/// Python virtual environment events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PythonEvent {
    /// Python virtual environment creating
    VenvCreating {
        package: String,
        version: Version,
        venv_path: String,
    },

    /// Python virtual environment created
    VenvCreated {
        package: String,
        version: Version,
        venv_path: String,
    },

    /// Python wheel installing
    WheelInstalling {
        package: String,
        version: Version,
        wheel_file: String,
    },

    /// Python wheel installed
    WheelInstalled {
        package: String,
        version: Version,
    },

    /// Python wrapper creating
    WrapperCreating {
        package: String,
        executable: String,
        wrapper_path: String,
    },

    /// Python wrapper created
    WrapperCreated {
        package: String,
        executable: String,
        wrapper_path: String,
    },

    /// Python virtual environment cloning
    VenvCloning {
        package: String,
        version: Version,
        from_path: String,
        to_path: String,
    },

    /// Python virtual environment cloned
    VenvCloned {
        package: String,
        version: Version,
        from_path: String,
        to_path: String,
    },

    /// Python virtual environment removing
    VenvRemoving {
        package: String,
        version: Version,
        venv_path: String,
    },

    /// Python virtual environment removed
    VenvRemoved {
        package: String,
        version: Version,
        venv_path: String,
    },
}