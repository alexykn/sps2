//! Environment verification and coordination

use super::core::BuildEnvironment;
use sps2_errors::{BuildError, Error};

impl BuildEnvironment {
    /// Verify build environment isolation is properly set up
    ///
    /// # Errors
    ///
    /// Returns an error if the build environment is not properly isolated or directories are missing.
    pub fn verify_isolation(&self) -> Result<(), Error> {
        // Check that critical directories exist
        if !self.build_prefix.exists() {
            return Err(BuildError::Failed {
                message: format!(
                    "Build prefix does not exist: {}",
                    self.build_prefix.display()
                ),
            }
            .into());
        }

        if !self.staging_dir.exists() {
            return Err(BuildError::Failed {
                message: format!(
                    "Staging directory does not exist: {}",
                    self.staging_dir.display()
                ),
            }
            .into());
        }

        // Verify environment variables are set correctly
        let required_vars = vec!["PREFIX", "DESTDIR", "JOBS"];
        for var in required_vars {
            if !self.env_vars.contains_key(var) {
                return Err(BuildError::Failed {
                    message: format!("Required environment variable {var} not set"),
                }
                .into());
            }
        }

        // PATH will be updated when build dependencies are installed
        // So we just check it exists for now
        if !self.env_vars.contains_key("PATH") {
            return Err(BuildError::Failed {
                message: "PATH environment variable not set".to_string(),
            }
            .into());
        }

        Ok(())
    }
}
