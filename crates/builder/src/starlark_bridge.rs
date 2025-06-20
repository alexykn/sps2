//! Bridge implementation for Starlark API integration

use crate::api::BuilderApi;
use crate::BuildEnvironment;
use sps2_errors::Error;
use sps2_package::BuildExecutor;
use sps2_types::RpathStyle;
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
    async fn fetch(&mut self, url: &str) -> Result<PathBuf, Error> {
        self.api.fetch(url).await
    }

    async fn fetch_md5(&mut self, url: &str, expected_md5: &str) -> Result<PathBuf, Error> {
        self.api.fetch_md5(url, expected_md5).await
    }

    async fn fetch_sha256(&mut self, url: &str, expected_sha256: &str) -> Result<PathBuf, Error> {
        self.api.fetch_sha256(url, expected_sha256).await
    }

    async fn fetch_blake3(&mut self, url: &str, expected_blake3: &str) -> Result<PathBuf, Error> {
        self.api.fetch_blake3(url, expected_blake3).await
    }

    async fn git(&mut self, url: &str, ref_: &str) -> Result<PathBuf, Error> {
        self.api.git(url, ref_).await
    }

    async fn make(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.make(args, &mut self.env).await?;
        Ok(())
    }

    async fn install(&mut self) -> Result<(), Error> {
        self.api.install(&self.env).await?;
        Ok(())
    }

    async fn configure(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.configure(args, &mut self.env).await?;
        Ok(())
    }

    async fn autotools(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.autotools(args, &mut self.env).await?;
        Ok(())
    }

    async fn cmake(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.cmake(args, &mut self.env).await?;
        Ok(())
    }

    async fn meson(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.meson(args, &mut self.env).await?;
        Ok(())
    }

    async fn cargo(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.cargo(args, &mut self.env).await?;
        Ok(())
    }

    async fn go(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.go(args, &mut self.env).await?;
        Ok(())
    }

    async fn python(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.python(args, &mut self.env).await?;
        Ok(())
    }

    async fn nodejs(&mut self, args: &[String]) -> Result<(), Error> {
        self.api.nodejs(args, &mut self.env).await?;
        Ok(())
    }

    async fn apply_patch(&mut self, patch_path: &Path) -> Result<(), Error> {
        self.api.apply_patch(patch_path, &self.env).await?;
        Ok(())
    }

    async fn copy(&mut self, src_path: Option<&str>) -> Result<(), Error> {
        self.api.copy(src_path, &self.env.context).await?;
        Ok(())
    }

    async fn with_defaults(&mut self) -> Result<(), Error> {
        // Apply default compiler flags to the build environment
        self.env.apply_default_compiler_flags();
        Ok(())
    }

    async fn patch_rpaths(&mut self, style: RpathStyle, paths: &[String]) -> Result<(), Error> {
        self.api.patch_rpaths(style, paths, &self.env).await?;
        Ok(())
    }

    async fn fix_permissions(&mut self, paths: &[String]) -> Result<(), Error> {
        self.api.fix_permissions(paths, &mut self.env).await?;
        Ok(())
    }
}
