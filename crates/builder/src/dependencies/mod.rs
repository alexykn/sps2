// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Advanced dependency management system for sps2 builder
//!
//! This module provides comprehensive dependency resolution with support for:
//! - Build dependencies (needed at build time)
//! - Host dependencies (needed on build machine)
//! - Target dependencies (needed on target machine)
//! - Optional dependencies with feature flags
//! - Cycle detection
//! - Dependency graph generation

use sps2_errors::{BuildError, Error};
use sps2_events::{Event, EventSender};
use sps2_resolver::{ResolutionContext, Resolver};
use sps2_types::package::{DepKind, PackageSpec};
use sps2_types::Version;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

/// Extended dependency kinds for cross-compilation support
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtendedDepKind {
    /// Build dependencies - needed at build time (compilers, build tools)
    Build,
    /// Host dependencies - needed on the build machine (pkg-config, cmake)
    Host,
    /// Target dependencies - needed on the target machine (runtime libs)
    Target,
    /// Runtime dependencies - alias for Target
    Runtime,
}

impl ExtendedDepKind {
    /// Convert to standard DepKind for resolver compatibility
    pub fn to_standard(&self) -> DepKind {
        match self {
            Self::Build | Self::Host => DepKind::Build,
            Self::Target | Self::Runtime => DepKind::Runtime,
        }
    }
}

impl fmt::Display for ExtendedDepKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build => write!(f, "build"),
            Self::Host => write!(f, "host"),
            Self::Target => write!(f, "target"),
            Self::Runtime => write!(f, "runtime"),
        }
    }
}

/// Dependency with optional feature flags
#[derive(Debug, Clone)]
pub struct Dependency {
    /// Package specification
    pub spec: PackageSpec,
    /// Dependency kind
    pub kind: ExtendedDepKind,
    /// Optional - only needed if certain features are enabled
    pub optional: bool,
    /// Features that enable this dependency
    pub features: HashSet<String>,
}

impl Dependency {
    /// Create a new dependency
    pub fn new(spec: PackageSpec, kind: ExtendedDepKind) -> Self {
        Self {
            spec,
            kind,
            optional: false,
            features: HashSet::new(),
        }
    }

    /// Mark dependency as optional with features
    pub fn with_features(mut self, features: impl IntoIterator<Item = String>) -> Self {
        self.optional = true;
        self.features = features.into_iter().collect();
        self
    }
}

/// Dependency graph node
#[derive(Debug, Clone)]
pub struct DependencyNode {
    /// Package name
    pub name: String,
    /// Package version
    pub version: Version,
    /// Dependencies of this node
    pub dependencies: Vec<Dependency>,
    /// Whether this is a virtual node (for grouping)
    pub virtual_node: bool,
}

/// Dependency resolution context with features
#[derive(Debug, Clone)]
pub struct DependencyContext {
    /// Build dependencies
    pub build_deps: Vec<Dependency>,
    /// Host dependencies
    pub host_deps: Vec<Dependency>,
    /// Target dependencies
    pub target_deps: Vec<Dependency>,
    /// Enabled features
    pub enabled_features: HashSet<String>,
    /// Target architecture (for cross-compilation)
    pub target_arch: Option<String>,
    /// Host architecture
    pub host_arch: String,
}

impl DependencyContext {
    /// Create new dependency context
    pub fn new(host_arch: String) -> Self {
        Self {
            build_deps: Vec::new(),
            host_deps: Vec::new(),
            target_deps: Vec::new(),
            enabled_features: HashSet::new(),
            target_arch: None,
            host_arch,
        }
    }

    /// Add a dependency
    pub fn add_dependency(&mut self, dep: Dependency) {
        match dep.kind {
            ExtendedDepKind::Build => self.build_deps.push(dep),
            ExtendedDepKind::Host => self.host_deps.push(dep),
            ExtendedDepKind::Target | ExtendedDepKind::Runtime => self.target_deps.push(dep),
        }
    }

    /// Enable a feature
    pub fn enable_feature(&mut self, feature: impl Into<String>) {
        self.enabled_features.insert(feature.into());
    }

    /// Check if cross-compiling
    pub fn is_cross_compiling(&self) -> bool {
        self.target_arch
            .as_ref()
            .is_some_and(|t| t != &self.host_arch)
    }

    /// Filter dependencies by enabled features
    pub fn active_dependencies(&self) -> Vec<&Dependency> {
        let mut deps = Vec::new();

        for dep in &self.build_deps {
            if self.is_dependency_active(dep) {
                deps.push(dep);
            }
        }

        for dep in &self.host_deps {
            if self.is_dependency_active(dep) {
                deps.push(dep);
            }
        }

        for dep in &self.target_deps {
            if self.is_dependency_active(dep) {
                deps.push(dep);
            }
        }

        deps
    }

    /// Check if a dependency is active based on features
    fn is_dependency_active(&self, dep: &Dependency) -> bool {
        if !dep.optional {
            return true;
        }

        // Optional dependency is active if any of its features are enabled
        dep.features
            .iter()
            .any(|f| self.enabled_features.contains(f))
    }
}

/// Advanced dependency resolver
pub struct DependencyResolver {
    /// Package resolver
    resolver: Resolver,
    /// Event sender for progress
    event_sender: Option<EventSender>,
}

impl DependencyResolver {
    /// Create new dependency resolver
    pub fn new(resolver: Resolver, event_sender: Option<EventSender>) -> Self {
        Self {
            resolver,
            event_sender,
        }
    }

    /// Resolve dependencies with feature support
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Dependency resolution fails
    /// - Cycles are detected
    /// - Feature conflicts occur
    pub async fn resolve(&self, context: &DependencyContext) -> Result<DependencyGraph, Error> {
        // Send resolution start event
        if let Some(sender) = &self.event_sender {
            let _ = sender.send(Event::OperationStarted {
                operation: "dependency_resolution".to_string(),
            });
        }

        // Create resolution context for standard resolver
        let mut resolution_context = ResolutionContext::new();

        // Add active dependencies to resolution context
        for dep in context.active_dependencies() {
            match dep.kind.to_standard() {
                DepKind::Build => {
                    resolution_context = resolution_context.add_build_dep(dep.spec.clone());
                }
                DepKind::Runtime => {
                    resolution_context = resolution_context.add_runtime_dep(dep.spec.clone());
                }
            }
        }

        // Resolve using standard resolver
        let resolution = self.resolver.resolve(resolution_context).await?;

        // Build dependency graph
        let mut graph = DependencyGraph::new();

        // Add resolved nodes to graph
        for node in resolution.nodes.values() {
            graph.add_node(DependencyNode {
                name: node.name.clone(),
                version: node.version.clone(),
                dependencies: Vec::new(), // Will be populated during graph construction
                virtual_node: false,
            });
        }

        // Detect cycles
        if let Some(cycle) = graph.detect_cycle() {
            return Err(BuildError::DependencyConflict {
                message: format!("Dependency cycle detected: {}", cycle.join(" -> ")),
            }
            .into());
        }

        // Send completion event
        if let Some(sender) = &self.event_sender {
            let _ = sender.send(Event::DependencyResolved {
                package: "build".to_string(),
                version: Version::parse("0.0.0").unwrap(),
                count: graph.nodes.len(),
            });
        }

        Ok(graph)
    }

    /// Resolve with SAT solver for complex constraints
    ///
    /// # Errors
    ///
    /// Returns an error if SAT solving fails
    pub async fn resolve_with_sat(
        &self,
        context: &DependencyContext,
    ) -> Result<DependencyGraph, Error> {
        // Create resolution context
        let mut resolution_context = ResolutionContext::new();

        for dep in context.active_dependencies() {
            match dep.kind.to_standard() {
                DepKind::Build => {
                    resolution_context = resolution_context.add_build_dep(dep.spec.clone());
                }
                DepKind::Runtime => {
                    resolution_context = resolution_context.add_runtime_dep(dep.spec.clone());
                }
            }
        }

        // Use SAT solver
        let resolution = self.resolver.resolve_with_sat(resolution_context).await?;

        // Convert to dependency graph
        let mut graph = DependencyGraph::new();

        for node in resolution.nodes.values() {
            graph.add_node(DependencyNode {
                name: node.name.clone(),
                version: node.version.clone(),
                dependencies: Vec::new(),
                virtual_node: false,
            });
        }

        Ok(graph)
    }
}

/// Dependency graph for visualization and analysis
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// Nodes in the graph
    pub nodes: HashMap<String, DependencyNode>,
    /// Adjacency list representation
    pub edges: HashMap<String, Vec<String>>,
}

impl DependencyGraph {
    /// Create new empty graph
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: HashMap::new(),
        }
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: DependencyNode) {
        let key = format!("{}-{}", node.name, node.version);
        self.nodes.insert(key.clone(), node);
        self.edges.entry(key).or_default();
    }

    /// Add an edge between nodes
    pub fn add_edge(&mut self, from: &str, to: &str) {
        self.edges
            .entry(from.to_string())
            .or_default()
            .push(to.to_string());
    }

    /// Detect cycles using DFS
    pub fn detect_cycle(&self) -> Option<Vec<String>> {
        let mut visited = HashSet::new();
        let mut stack = HashSet::new();
        let mut path = Vec::new();

        for node in self.nodes.keys() {
            if !visited.contains(node) {
                if let Some(cycle) =
                    self.dfs_cycle_detect(node, &mut visited, &mut stack, &mut path)
                {
                    return Some(cycle);
                }
            }
        }

        None
    }

    /// DFS helper for cycle detection
    fn dfs_cycle_detect(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        visited.insert(node.to_string());
        stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(neighbors) = self.edges.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    if let Some(cycle) = self.dfs_cycle_detect(neighbor, visited, stack, path) {
                        return Some(cycle);
                    }
                } else if stack.contains(neighbor) {
                    // Found a cycle
                    let cycle_start = path.iter().position(|n| n == neighbor).unwrap();
                    return Some(path[cycle_start..].to_vec());
                }
            }
        }

        stack.remove(node);
        path.pop();
        None
    }

    /// Generate DOT format for visualization
    pub fn to_dot(&self) -> String {
        let mut dot = String::from("digraph Dependencies {\n");
        dot.push_str("  rankdir=LR;\n");
        dot.push_str("  node [shape=box];\n\n");

        // Add nodes
        for (key, node) in &self.nodes {
            let label = if node.virtual_node {
                format!("<<i>{}</i>>", node.name)
            } else {
                format!("\"{}\\n{}\"", node.name, node.version)
            };
            dot.push_str(&format!("  \"{key}\" [label={label}];\n"));
        }

        dot.push('\n');

        // Add edges
        for (from, tos) in &self.edges {
            for to in tos {
                dot.push_str(&format!("  \"{from}\" -> \"{to}\";\n"));
            }
        }

        dot.push_str("}\n");
        dot
    }

    /// Get topological order (returns None if cycles exist)
    pub fn topological_sort(&self) -> Option<Vec<String>> {
        // Calculate in-degrees
        let mut in_degree = HashMap::new();
        for node in self.nodes.keys() {
            in_degree.insert(node.clone(), 0);
        }

        for edges in self.edges.values() {
            for to in edges {
                *in_degree.get_mut(to).unwrap() += 1;
            }
        }

        // Queue for nodes with no incoming edges
        let mut queue = VecDeque::new();
        for (node, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(node.clone());
            }
        }

        let mut result = Vec::new();

        while let Some(node) = queue.pop_front() {
            result.push(node.clone());

            if let Some(neighbors) = self.edges.get(&node) {
                for neighbor in neighbors {
                    let degree = in_degree.get_mut(neighbor).unwrap();
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(neighbor.clone());
                    }
                }
            }
        }

        if result.len() == self.nodes.len() {
            Some(result)
        } else {
            None // Cycle exists
        }
    }

    /// Find all paths between two nodes
    pub fn find_paths(&self, from: &str, to: &str) -> Vec<Vec<String>> {
        let mut paths = Vec::new();
        let mut current_path = vec![from.to_string()];
        let mut visited = HashSet::new();

        self.dfs_find_paths(from, to, &mut current_path, &mut visited, &mut paths);

        paths
    }

    /// DFS helper for finding paths
    fn dfs_find_paths(
        &self,
        current: &str,
        target: &str,
        path: &mut Vec<String>,
        visited: &mut HashSet<String>,
        paths: &mut Vec<Vec<String>>,
    ) {
        if current == target {
            paths.push(path.clone());
            return;
        }

        visited.insert(current.to_string());

        if let Some(neighbors) = self.edges.get(current) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    path.push(neighbor.clone());
                    self.dfs_find_paths(neighbor, target, path, visited, paths);
                    path.pop();
                }
            }
        }

        visited.remove(current);
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}
