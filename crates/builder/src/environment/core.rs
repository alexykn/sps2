//! Core `BuildEnvironment` struct and construction

use crate::BuildContext;
use sps2_errors::Error;
use sps2_install::Installer;
use sps2_net::NetClient;
use sps2_resolver::Resolver;
use sps2_store::PackageStore;
use sps2_types::Version;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Live prefix where packages are installed at runtime
pub const LIVE_PREFIX: &str = "/opt/pm/live";

/// Build environment for isolated package building
#[derive(Clone, Debug)]
pub struct BuildEnvironment {
    /// Build context
    pub(crate) context: BuildContext,
    /// Build prefix directory
    pub(crate) build_prefix: PathBuf,
    /// Staging directory for installation
    pub(crate) staging_dir: PathBuf,
    /// Environment variables
    pub(crate) env_vars: HashMap<String, String>,
    /// Build metadata from build systems (e.g., Python wheel path)
    pub(crate) build_metadata: HashMap<String, String>,
    /// Resolver for dependencies
    pub(crate) resolver: Option<Resolver>,
    /// Package store for build dependencies
    pub(crate) store: Option<PackageStore>,
    /// Installer for build dependencies
    pub(crate) installer: Option<Installer>,
    /// Network client for downloads
    pub(crate) net: Option<NetClient>,
    /// Whether with_defaults() was called (for optimized builds)
    pub(crate) with_defaults_called: bool,
    /// Build systems used during the build process
    pub(crate) used_build_systems: HashSet<String>,
    /// Fix permissions requests (None if not requested, Some(paths) if requested)
    pub(crate) fix_permissions_request: Option<Vec<String>>,
}

impl BuildEnvironment {
    /// Create new build environment
    ///
    /// # Errors
    ///
    /// Returns an error if the build environment cannot be initialized.
    pub fn new(context: BuildContext, build_root: &Path) -> Result<Self, Error> {
        let build_prefix = Self::get_build_prefix_path(build_root, &context.name, &context.version);
        let staging_dir = build_prefix.join("stage");

        let mut env_vars = HashMap::new();
        env_vars.insert("PREFIX".to_string(), "/opt/pm/live".to_string());
        env_vars.insert("DESTDIR".to_string(), staging_dir.display().to_string());
        env_vars.insert("JOBS".to_string(), Self::cpu_count().to_string());

        Ok(Self {
            context,
            build_prefix,
            staging_dir,
            env_vars,
            build_metadata: HashMap::new(),
            resolver: None,
            store: None,
            installer: None,
            net: None,
            with_defaults_called: false,
            used_build_systems: HashSet::new(),
            fix_permissions_request: None,
        })
    }

    /// Set resolver for dependency management
    #[must_use]
    pub fn with_resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Set package store for build dependencies
    #[must_use]
    pub fn with_store(mut self, store: PackageStore) -> Self {
        self.store = Some(store);
        self
    }

    /// Set installer for build dependencies
    #[must_use]
    pub fn with_installer(mut self, installer: Installer) -> Self {
        self.installer = Some(installer);
        self
    }

    /// Set network client for downloads
    #[must_use]
    pub fn with_net(mut self, net: NetClient) -> Self {
        self.net = Some(net);
        self
    }

    /// Get staging directory
    #[must_use]
    pub fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }

    /// Get build context
    #[must_use]
    pub fn context(&self) -> &BuildContext {
        &self.context
    }

    /// Get build prefix
    #[must_use]
    pub fn build_prefix(&self) -> &Path {
        &self.build_prefix
    }

    /// Get BUILD_PREFIX environment variable value (package-specific prefix)
    #[must_use]
    pub fn get_build_prefix(&self) -> String {
        format!("/{}-{}", self.context.name, self.context.version)
    }

    /// Get the live prefix where packages are installed at runtime
    #[must_use]
    pub fn get_live_prefix(&self) -> &'static str {
        LIVE_PREFIX
    }

    /// Get environment variables
    #[must_use]
    pub fn env_vars(&self) -> &HashMap<String, String> {
        &self.env_vars
    }

    /// Set environment variable
    ///
    /// # Errors
    ///
    /// Currently infallible, but returns Result for future compatibility.
    pub fn set_env_var(&mut self, key: String, value: String) -> Result<(), Error> {
        self.env_vars.insert(key, value);
        Ok(())
    }

    /// Get the package path from the build context
    #[must_use]
    pub fn package_path(&self) -> Option<&Path> {
        self.context.package_path.as_deref()
    }

    /// Get the output path where the package will be created
    #[must_use]
    pub fn package_output_path(&self) -> PathBuf {
        self.context.output_path()
    }

    /// Check if this is a Python package based on build metadata
    #[must_use]
    pub fn is_python_package(&self) -> bool {
        self.build_metadata.contains_key("PYTHON_WHEEL_PATH")
            || self.build_metadata.contains_key("PYTHON_BUILD_BACKEND")
    }

    /// Get extra environment variable (checks build_metadata first, then env_vars)
    #[must_use]
    pub fn get_extra_env(&self, key: &str) -> Option<String> {
        self.build_metadata
            .get(key)
            .cloned()
            .or_else(|| self.env_vars.get(key).cloned())
    }

    /// Set build metadata
    pub fn set_build_metadata(&mut self, key: String, value: String) {
        self.build_metadata.insert(key, value);
    }

    /// Get all build metadata
    #[must_use]
    pub fn build_metadata(&self) -> &HashMap<String, String> {
        &self.build_metadata
    }

    /// Record that a build system was used during the build
    pub fn record_build_system(&mut self, build_system: &str) {
        self.used_build_systems.insert(build_system.to_string());
    }

    /// Get all build systems used during the build
    #[must_use]
    pub fn used_build_systems(&self) -> &HashSet<String> {
        &self.used_build_systems
    }

    /// Get package name
    #[must_use]
    pub fn package_name(&self) -> &str {
        &self.context.name
    }

    /// Record that fix_permissions was requested
    pub fn record_fix_permissions_request(&mut self, paths: Vec<String>) {
        // If already requested, merge the paths
        if let Some(existing_paths) = &mut self.fix_permissions_request {
            existing_paths.extend(paths);
        } else {
            self.fix_permissions_request = Some(paths);
        }
    }

    /// Get build prefix path for package
    #[must_use]
    pub(crate) fn get_build_prefix_path(
        build_root: &Path,
        name: &str,
        version: &Version,
    ) -> PathBuf {
        build_root.join(name).join(version.to_string())
    }

    /// Get CPU count for parallel builds
    #[must_use]
    pub(crate) fn cpu_count() -> usize {
        // Use 75% of available cores as per specification
        let cores = num_cpus::get();
        let target = cores.saturating_mul(3).saturating_add(3) / 4; // 75% using integer arithmetic
        std::cmp::max(1, target)
    }
}
