#![deny(clippy::pedantic)]
#![allow(clippy::module_name_repetitions, unsafe_code)]

//! Package installation with atomic updates for spsv2
//!
//! This crate handles the installation of packages with atomic
//! state transitions, rollback capabilities, and parallel execution.

mod atomic;
mod installer;
mod operations;
mod parallel;

pub use atomic::{AtomicInstaller, StateTransition};
pub use installer::{InstallConfig, Installer};
pub use operations::{InstallOperation, UninstallOperation, UpdateOperation};
pub use parallel::{ExecutionContext, ParallelExecutor};

use spsv2_errors::Error;
use spsv2_events::EventSender;
use spsv2_resolver::{PackageId, ResolutionResult};
use spsv2_types::{PackageSpec, Version};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Installation context
#[derive(Clone, Debug)]
pub struct InstallContext {
    /// Package specifications to install
    pub packages: Vec<PackageSpec>,
    /// Local package files to install
    pub local_files: Vec<PathBuf>,
    /// Force reinstallation
    pub force: bool,
    /// Dry run mode
    pub dry_run: bool,
    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

impl InstallContext {
    /// Create new install context
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            local_files: Vec::new(),
            force: false,
            dry_run: false,
            event_sender: None,
        }
    }

    /// Add package to install
    pub fn add_package(mut self, spec: PackageSpec) -> Self {
        self.packages.push(spec);
        self
    }

    /// Add local file to install
    pub fn add_local_file(mut self, path: PathBuf) -> Self {
        self.local_files.push(path);
        self
    }

    /// Set force reinstallation
    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    /// Set dry run mode
    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Set event sender
    pub fn with_event_sender(mut self, event_sender: EventSender) -> Self {
        self.event_sender = Some(event_sender);
        self
    }
}

impl Default for InstallContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Installation result
#[derive(Debug)]
pub struct InstallResult {
    /// State ID after installation
    pub state_id: Uuid,
    /// Packages that were installed
    pub installed_packages: Vec<PackageId>,
    /// Packages that were updated
    pub updated_packages: Vec<PackageId>,
    /// Packages that were removed
    pub removed_packages: Vec<PackageId>,
}

impl InstallResult {
    /// Create new install result
    pub fn new(state_id: Uuid) -> Self {
        Self {
            state_id,
            installed_packages: Vec::new(),
            updated_packages: Vec::new(),
            removed_packages: Vec::new(),
        }
    }

    /// Add installed package
    pub fn add_installed(&mut self, package_id: PackageId) {
        self.installed_packages.push(package_id);
    }

    /// Add updated package
    pub fn add_updated(&mut self, package_id: PackageId) {
        self.updated_packages.push(package_id);
    }

    /// Add removed package
    pub fn add_removed(&mut self, package_id: PackageId) {
        self.removed_packages.push(package_id);
    }

    /// Get total number of changes
    pub fn total_changes(&self) -> usize {
        self.installed_packages.len() + self.updated_packages.len() + self.removed_packages.len()
    }
}

/// Uninstall context
#[derive(Clone, Debug)]
pub struct UninstallContext {
    /// Package names to uninstall
    pub packages: Vec<String>,
    /// Remove dependencies if no longer needed
    pub autoremove: bool,
    /// Force removal even with dependents
    pub force: bool,
    /// Dry run mode
    pub dry_run: bool,
    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

impl UninstallContext {
    /// Create new uninstall context
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            autoremove: false,
            force: false,
            dry_run: false,
            event_sender: None,
        }
    }

    /// Add package to uninstall
    pub fn add_package(mut self, name: String) -> Self {
        self.packages.push(name);
        self
    }

    /// Set autoremove
    pub fn with_autoremove(mut self, autoremove: bool) -> Self {
        self.autoremove = autoremove;
        self
    }

    /// Set force removal
    pub fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    /// Set dry run mode
    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Set event sender
    pub fn with_event_sender(mut self, event_sender: EventSender) -> Self {
        self.event_sender = Some(event_sender);
        self
    }
}

impl Default for UninstallContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Update context
#[derive(Clone, Debug)]
pub struct UpdateContext {
    /// Packages to update (empty = all)
    pub packages: Vec<String>,
    /// Upgrade mode (ignore upper bounds)
    pub upgrade: bool,
    /// Dry run mode
    pub dry_run: bool,
    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

impl UpdateContext {
    /// Create new update context
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            upgrade: false,
            dry_run: false,
            event_sender: None,
        }
    }

    /// Add package to update
    pub fn add_package(mut self, name: String) -> Self {
        self.packages.push(name);
        self
    }

    /// Set upgrade mode
    pub fn with_upgrade(mut self, upgrade: bool) -> Self {
        self.upgrade = upgrade;
        self
    }

    /// Set dry run mode
    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Set event sender
    pub fn with_event_sender(mut self, event_sender: EventSender) -> Self {
        self.event_sender = Some(event_sender);
        self
    }
}

impl Default for UpdateContext {
    fn default() -> Self {
        Self::new()
    }
}
