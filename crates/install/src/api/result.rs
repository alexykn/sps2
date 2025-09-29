use chrono::{DateTime, Utc};
use sps2_resolver::PackageId;
use uuid::Uuid;

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
    #[must_use]
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
    #[must_use]
    pub fn total_changes(&self) -> usize {
        self.installed_packages.len() + self.updated_packages.len() + self.removed_packages.len()
    }
}

/// State information for listing
#[derive(Debug, Clone)]
pub struct StateInfo {
    /// State ID
    pub id: Uuid,
    /// Creation timestamp
    pub timestamp: DateTime<Utc>,
    /// Parent state ID
    pub parent_id: Option<Uuid>,
    /// Number of packages in this state
    pub package_count: usize,
    /// Sample of packages (for display)
    pub packages: Vec<sps2_types::PackageId>,
}

impl StateInfo {
    /// Check if this is the root state
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.parent_id.is_none()
    }

    /// Get age of this state
    #[must_use]
    pub fn age(&self) -> chrono::Duration {
        Utc::now() - self.timestamp
    }

    /// Format package list for display
    #[must_use]
    pub fn package_summary(&self) -> String {
        if self.packages.is_empty() {
            "No packages".to_string()
        } else if self.packages.len() <= 3 {
            self.packages
                .iter()
                .map(|pkg| format!("{}-{}", pkg.name, pkg.version))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            let first_three: Vec<String> = self
                .packages
                .iter()
                .take(3)
                .map(|pkg| format!("{}-{}", pkg.name, pkg.version))
                .collect();
            format!(
                "{} and {} more",
                first_three.join(", "),
                self.package_count - 3
            )
        }
    }
}
