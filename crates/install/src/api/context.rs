use sps2_events::EventSender;
use sps2_types::PackageSpec;
use std::path::PathBuf;

/// Installation context
#[derive(Clone, Debug)]
pub struct InstallContext {
    /// Package specifications to install
    pub packages: Vec<PackageSpec>,
    /// Local package files to install
    pub local_files: Vec<PathBuf>,
    /// Force reinstallation
    pub force: bool,

    /// Force re-download even if cached in the store
    pub force_download: bool,

    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

context_builder! {
    InstallContext {
        packages: Vec<PackageSpec>,
        local_files: Vec<PathBuf>,
        force: bool,
        force_download: bool,

    }
}
context_add_package_method!(InstallContext, PackageSpec);

/// Uninstall context
#[derive(Clone, Debug)]
pub struct UninstallContext {
    /// Package names to uninstall
    pub packages: Vec<String>,
    /// Remove dependencies if no longer needed
    pub autoremove: bool,
    /// Force removal even with dependents
    pub force: bool,

    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

context_builder! {
    UninstallContext {
        packages: Vec<String>,
        autoremove: bool,
        force: bool,

    }
}
context_add_package_method!(UninstallContext, String);

/// Update context
#[derive(Clone, Debug)]
pub struct UpdateContext {
    /// Packages to update (empty = all)
    pub packages: Vec<String>,
    /// Upgrade mode (ignore upper bounds)
    pub upgrade: bool,

    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

context_builder! {
    UpdateContext {
        packages: Vec<String>,
        upgrade: bool,

    }
}
context_add_package_method!(UpdateContext, String);
