//! Mapping between package versions and SAT variables

use super::{PackageVersion, Variable};
use std::collections::HashMap;

/// Maps package versions to SAT variables
#[derive(Debug, Clone)]
pub struct VariableMap {
    /// Next variable index to allocate
    next_var: u32,
    /// Map from package version to variable
    package_to_var: HashMap<PackageVersion, Variable>,
    /// Reverse map from variable to package version
    var_to_package: HashMap<Variable, PackageVersion>,
    /// Map from package name to all its versions
    package_versions: HashMap<String, Vec<PackageVersion>>,
}

impl VariableMap {
    /// Create new empty variable map
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_var: 0,
            package_to_var: HashMap::new(),
            var_to_package: HashMap::new(),
            package_versions: HashMap::new(),
        }
    }

    /// Add a package version and return its variable
    pub fn add_package_version(&mut self, package: PackageVersion) -> Variable {
        // Check if already exists
        if let Some(&var) = self.package_to_var.get(&package) {
            return var;
        }

        // Allocate new variable
        let var = Variable::new(self.next_var);
        self.next_var += 1;

        // Add to maps
        self.package_to_var.insert(package.clone(), var);
        self.var_to_package.insert(var, package.clone());

        // Add to package versions list
        self.package_versions
            .entry(package.name.clone())
            .or_default()
            .push(package);

        var
    }

    /// Get variable for a package version
    #[must_use]
    pub fn get_variable(&self, package: &PackageVersion) -> Option<Variable> {
        self.package_to_var.get(package).copied()
    }

    /// Get package version for a variable
    #[must_use]
    pub fn get_package(&self, var: Variable) -> Option<&PackageVersion> {
        self.var_to_package.get(&var)
    }

    /// Get all versions of a package
    #[must_use]
    pub fn get_package_versions(&self, name: &str) -> Vec<&PackageVersion> {
        self.package_versions
            .get(name)
            .map(|versions| versions.iter().collect())
            .unwrap_or_default()
    }

    /// Get all package names
    pub fn all_packages(&self) -> impl Iterator<Item = &str> {
        self.package_versions.keys().map(String::as_str)
    }

    /// Get number of variables
    #[must_use]
    pub fn num_variables(&self) -> u32 {
        self.next_var
    }

    /// Get all variables
    pub fn all_variables(&self) -> impl Iterator<Item = Variable> + '_ {
        (0..self.next_var).map(Variable::new)
    }

    /// Check if a package has any versions
    #[must_use]
    pub fn has_package(&self, name: &str) -> bool {
        self.package_versions.contains_key(name)
    }

    /// Get variables for all versions of a package
    #[must_use]
    pub fn get_package_variables(&self, name: &str) -> Vec<Variable> {
        self.get_package_versions(name)
            .into_iter()
            .filter_map(|pv| self.get_variable(pv))
            .collect()
    }

    /// Clear all mappings
    pub fn clear(&mut self) {
        self.next_var = 0;
        self.package_to_var.clear();
        self.var_to_package.clear();
        self.package_versions.clear();
    }
}

impl Default for VariableMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    #[test]
    fn test_variable_map() {
        let mut map = VariableMap::new();

        let pv1 = PackageVersion::new("foo".to_string(), Version::parse("1.0.0").unwrap());
        let pv2 = PackageVersion::new("foo".to_string(), Version::parse("2.0.0").unwrap());
        let pv3 = PackageVersion::new("bar".to_string(), Version::parse("1.0.0").unwrap());

        // Add package versions
        let v1 = map.add_package_version(pv1.clone());
        let v2 = map.add_package_version(pv2.clone());
        let v3 = map.add_package_version(pv3.clone());

        assert_eq!(v1.index(), 0);
        assert_eq!(v2.index(), 1);
        assert_eq!(v3.index(), 2);

        // Test retrieval
        assert_eq!(map.get_variable(&pv1), Some(v1));
        assert_eq!(map.get_variable(&pv2), Some(v2));
        assert_eq!(map.get_variable(&pv3), Some(v3));

        assert_eq!(map.get_package(v1), Some(&pv1));
        assert_eq!(map.get_package(v2), Some(&pv2));
        assert_eq!(map.get_package(v3), Some(&pv3));

        // Test package versions
        let foo_versions = map.get_package_versions("foo");
        assert_eq!(foo_versions.len(), 2);
        assert!(foo_versions.contains(&&pv1));
        assert!(foo_versions.contains(&&pv2));

        let bar_versions = map.get_package_versions("bar");
        assert_eq!(bar_versions.len(), 1);
        assert!(bar_versions.contains(&&pv3));

        // Test package variables
        let foo_vars = map.get_package_variables("foo");
        assert_eq!(foo_vars.len(), 2);
        assert!(foo_vars.contains(&v1));
        assert!(foo_vars.contains(&v2));

        // Test all packages
        let all_packages: Vec<&str> = map.all_packages().collect();
        assert_eq!(all_packages.len(), 2);
        assert!(all_packages.contains(&"foo"));
        assert!(all_packages.contains(&"bar"));

        // Test duplicate addition
        let v1_dup = map.add_package_version(pv1.clone());
        assert_eq!(v1_dup, v1); // Should return same variable

        assert_eq!(map.num_variables(), 3);
    }

    #[test]
    fn test_clear() {
        let mut map = VariableMap::new();

        let pv = PackageVersion::new("test".to_string(), Version::parse("1.0.0").unwrap());

        map.add_package_version(pv.clone());
        assert_eq!(map.num_variables(), 1);
        assert!(map.has_package("test"));

        map.clear();
        assert_eq!(map.num_variables(), 0);
        assert!(!map.has_package("test"));
    }
}
