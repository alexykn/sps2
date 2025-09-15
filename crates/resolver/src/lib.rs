#![deny(clippy::pedantic, unsafe_code)]
#![allow(clippy::module_name_repetitions)]

//! Dependency resolution for sps2
//!
//! This crate provides deterministic, parallel dependency resolution
//! for both installation and building operations. It implements a
//! topological sort with concurrent execution.

mod execution;
mod graph;
mod resolver;
mod sat;

pub use execution::ExecutionPlan;
pub use graph::{DepEdge, DepKind, DependencyGraph, NodeAction, PackageId, ResolvedNode};
pub use resolver::Resolver;
pub use sat::{solve_dependencies, DependencyProblem, DependencySolution};

use sps2_types::package::PackageSpec;
use sps2_types::Version;
use std::collections::HashMap;
use std::path::PathBuf;

/// Simple representation of an installed package
#[derive(Clone, Debug)]
pub struct InstalledPackage {
    /// Package name
    pub name: String,
    /// Package version
    pub version: Version,
}

impl InstalledPackage {
    /// Create new installed package
    #[must_use]
    pub fn new(name: String, version: Version) -> Self {
        Self { name, version }
    }
}

/// Resolution context for packages
#[derive(Clone, Debug)]
pub struct ResolutionContext {
    /// Runtime dependencies to resolve
    pub runtime_deps: Vec<PackageSpec>,
    /// Build dependencies to resolve (only for build operations)
    pub build_deps: Vec<PackageSpec>,
    /// Local package files to include
    pub local_files: Vec<PathBuf>,
    /// Already installed packages that can satisfy dependencies
    pub installed_packages: Vec<InstalledPackage>,
}

impl ResolutionContext {
    /// Create new resolution context
    #[must_use]
    pub fn new() -> Self {
        Self {
            runtime_deps: Vec::new(),
            build_deps: Vec::new(),
            local_files: Vec::new(),
            installed_packages: Vec::new(),
        }
    }

    /// Add runtime dependency
    #[must_use]
    pub fn add_runtime_dep(mut self, spec: PackageSpec) -> Self {
        self.runtime_deps.push(spec);
        self
    }

    /// Add build dependency
    #[must_use]
    pub fn add_build_dep(mut self, spec: PackageSpec) -> Self {
        self.build_deps.push(spec);
        self
    }

    /// Add local package file
    #[must_use]
    pub fn add_local_file(mut self, path: PathBuf) -> Self {
        self.local_files.push(path);
        self
    }

    /// Add installed packages
    #[must_use]
    pub fn with_installed_packages(mut self, packages: Vec<InstalledPackage>) -> Self {
        self.installed_packages = packages;
        self
    }
}

impl Default for ResolutionContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of dependency resolution
#[derive(Clone, Debug)]
pub struct ResolutionResult {
    /// Resolved dependency graph
    pub nodes: HashMap<PackageId, ResolvedNode>,
    /// Execution plan with topological order
    pub execution_plan: ExecutionPlan,
}

impl ResolutionResult {
    /// Get all packages in topological order
    #[must_use]
    pub fn packages_in_order(&self) -> Vec<&ResolvedNode> {
        self.execution_plan
            .batches()
            .iter()
            .flatten()
            .filter_map(|id| self.nodes.get(id))
            .collect()
    }

    /// Get packages by dependency kind
    #[must_use]
    pub fn packages_by_kind(&self, kind: &DepKind) -> Vec<&ResolvedNode> {
        self.nodes
            .values()
            .filter(|node| node.deps.iter().any(|edge| &edge.kind == kind))
            .collect()
    }
}
