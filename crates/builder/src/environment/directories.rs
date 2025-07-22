//! Build directory structure management

use super::core::BuildEnvironment;
use sps2_errors::{BuildError, Error};
use sps2_events::{AppEvent, EventEmitter};
use tokio::fs;

impl BuildEnvironment {
    /// Initialize the build environment
    ///
    /// # Errors
    ///
    /// Returns an error if directories cannot be created or environment setup fails.
    pub async fn initialize(&mut self) -> Result<(), Error> {
        self.emit_operation_started(format!(
            "Building {} {}",
            self.context.name, self.context.version
        ));

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

        self.emit_operation_completed(
            format!("Cleaned build environment for {}", self.context.name),
            true,
        );

        Ok(())
    }

    /// Send event if sender is available
    pub(crate) fn send_event(&self, event: AppEvent) {
        if let Some(sender) = self.event_sender() {
            sender.emit(event);
        }
    }
}
