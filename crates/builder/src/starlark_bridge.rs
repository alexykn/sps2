//! Bridge implementation for Starlark API integration

use crate::api::BuilderApi;
use crate::BuildEnvironment;
use sps2_errors::Error;
use sps2_package::BuildExecutor;
use std::path::{Path, PathBuf};

/// Bridge that implements `BuildExecutor` using `BuilderApi` and `BuildEnvironment`
pub struct StarlarkBridge {
    api: BuilderApi,
    env: BuildEnvironment,
}

impl std::fmt::Debug for StarlarkBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StarlarkBridge")
            .field("api", &"BuilderApi { ... }")
            .field("env", &"BuildEnvironment { ... }")
            .finish()
    }
}

impl StarlarkBridge {
    /// Create a new bridge with the given API and environment
    ///
    /// # Errors
    ///
    /// Returns an error if the `BuilderApi` cannot be created.
    pub fn new(working_dir: PathBuf, env: BuildEnvironment) -> Result<Self, Error> {
        Ok(Self {
            api: BuilderApi::new(working_dir)?,
            env,
        })
    }
}

#[async_trait::async_trait]
impl BuildExecutor for StarlarkBridge {
    async fn fetch(&mut self, url: &str, hash: &str) -> Result<PathBuf, Error> {
        self.api.fetch(url, hash).await
    }

    async fn make(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.make(args, &self.env).await?;
        Ok(())
    }

    async fn install(&mut self) -> Result<(), Error> {
        self.api.install(&self.env).await?;
        Ok(())
    }

    async fn configure(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.configure(args, &self.env).await?;
        Ok(())
    }

    async fn autotools(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.autotools(args, &self.env).await?;
        Ok(())
    }

    async fn cmake(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.cmake(args, &self.env).await?;
        Ok(())
    }

    async fn meson(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.meson(args, &self.env).await?;
        Ok(())
    }

    async fn cargo(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.cargo(args, &self.env).await?;
        Ok(())
    }

    async fn apply_patch(&mut self, patch_path: &Path) -> Result<(), Error> {
        self.api.apply_patch(patch_path, &self.env).await?;
        Ok(())
    }
}
