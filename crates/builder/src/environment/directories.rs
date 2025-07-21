//! Build directory structure management

use super::core::BuildEnvironment;
use sps2_errors::{BuildError, Error};
use sps2_events::{Event, EventEmitter};
use tokio::fs;

impl BuildEnvironment {
    /// Initialize the build environment
    ///
    /// # Errors
    ///
    /// Returns an error if directories cannot be created or environment setup fails.
    pub async fn initialize(&mut self) -> Result<(), Error> {
        self.send_event(Event::OperationStarted {
            operation: format!("Building {} {}", self.context.name, self.context.version),
        });

        // Create build directories with better error reporting
        fs::create_dir_all(&self.build_prefix)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!(
                    "Failed to create build prefix {}: {}",
                    self.build_prefix.display(),
                    e
                ),
            })?;

        fs::create_dir_all(&self.staging_dir)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!(
                    "Failed to create staging dir {}: {}",
                    self.staging_dir.display(),
                    e
                ),
            })?;

        // Set up environment variables
        self.setup_environment();

        Ok(())
    }

    /// Clean up build environment thoroughly
    ///
    /// # Errors
    ///
    /// Returns an error if directories cannot be removed during cleanup.
    pub async fn cleanup(&self) -> Result<(), Error> {
        // Remove any temporary build files in the build prefix
        let temp_dirs = vec!["src", "build", "tmp"];
        for dir in temp_dirs {
            let temp_path = self.build_prefix.join(dir);
            if temp_path.exists() {
                fs::remove_dir_all(&temp_path).await?;
            }
        }

        self.send_event(Event::OperationCompleted {
            operation: format!("Cleaned build environment for {}", self.context.name),
            success: true,
        });

        Ok(())
    }

    /// Send event if sender is available
    pub(crate) fn send_event(&self, event: Event) {
        self.emit_event(event);
    }
}
