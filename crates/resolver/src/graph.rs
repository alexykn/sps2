//! Dependency graph types and operations

use spsv2_types::{Version, VersionSpec};
use std::fmt;
use std::path::PathBuf;

/// Package identifier (name + version)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageId {
    pub name: String,
    pub version: Version,
}

impl PackageId {
    /// Create new package ID
    pub fn new(name: String, version: Version) -> Self {
        Self { name, version }
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.name, self.version)
    }
}

/// Dependency kind
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepKind {
    /// Build-time dependency
    Build,
    /// Runtime dependency
    Runtime,
}

/// Action to take for a resolved node
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeAction {
    /// Download binary package from repository
    Download,
    /// Use local package file
    Local,
}

/// Dependency edge in the resolution graph
#[derive(Clone, Debug)]
pub struct DepEdge {
    /// Package name
    pub name: String,
    /// Version specification
    pub spec: VersionSpec,
    /// Dependency kind
    pub kind: DepKind,
}

impl DepEdge {
    /// Create new dependency edge
    pub fn new(name: String, spec: VersionSpec, kind: DepKind) -> Self {
        Self { name, spec, kind }
    }

    /// Check if a version satisfies this edge
    pub fn satisfies(&self, version: &Version) -> bool {
        self.spec.matches(version)
    }
}

/// Resolved dependency node
#[derive(Clone, Debug)]
pub struct ResolvedNode {
    /// Package name
    pub name: String,
    /// Resolved version
    pub version: Version,
    /// Action to perform
    pub action: NodeAction,
    /// Dependencies of this package
    pub deps: Vec<DepEdge>,
    /// Download URL (for Download action)
    pub url: Option<String>,
    /// Local file path (for Local action)
    pub path: Option<PathBuf>,
}

impl ResolvedNode {
    /// Create new resolved node for download
    pub fn download(name: String, version: Version, url: String, deps: Vec<DepEdge>) -> Self {
        Self {
            name,
            version,
            action: NodeAction::Download,
            deps,
            url: Some(url),
            path: None,
        }
    }

    /// Create new resolved node for local file
    pub fn local(name: String, version: Version, path: PathBuf, deps: Vec<DepEdge>) -> Self {
        Self {
            name,
            version,
            action: NodeAction::Local,
            deps,
            url: None,
            path: Some(path),
        }
    }

    /// Get package ID
    pub fn package_id(&self) -> PackageId {
        PackageId::new(self.name.clone(), self.version.clone())
    }

    /// Get runtime dependencies
    pub fn runtime_deps(&self) -> impl Iterator<Item = &DepEdge> {
        self.deps
            .iter()
            .filter(|edge| edge.kind == DepKind::Runtime)
    }

    /// Get build dependencies
    pub fn build_deps(&self) -> impl Iterator<Item = &DepEdge> {
        self.deps.iter().filter(|edge| edge.kind == DepKind::Build)
    }
}

/// Dependency graph
#[derive(Clone, Debug)]
pub struct DependencyGraph {
    /// Resolved nodes indexed by package ID
    pub nodes: std::collections::HashMap<PackageId, ResolvedNode>,
    /// Adjacency list (package -> dependencies)
    pub edges: std::collections::HashMap<PackageId, Vec<PackageId>>,
}

impl DependencyGraph {
    /// Create new empty graph
    pub fn new() -> Self {
        Self {
            nodes: std::collections::HashMap::new(),
            edges: std::collections::HashMap::new(),
        }
    }

    /// Add node to graph
    pub fn add_node(&mut self, node: ResolvedNode) {
        let id = node.package_id();
        self.nodes.insert(id.clone(), node);
        self.edges.entry(id).or_default();
    }

    /// Add edge between two packages
    pub fn add_edge(&mut self, from: &PackageId, to: &PackageId) {
        self.edges.entry(from.clone()).or_default().push(to.clone());
    }

    /// Check for cycles using DFS
    pub fn has_cycles(&self) -> bool {
        use std::collections::HashSet;

        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for node_id in self.nodes.keys() {
            if !visited.contains(node_id)
                && self.has_cycle_util(node_id, &mut visited, &mut rec_stack)
            {
                return true;
            }
        }

        false
    }

    /// Utility function for cycle detection
    fn has_cycle_util(
        &self,
        node_id: &PackageId,
        visited: &mut std::collections::HashSet<PackageId>,
        rec_stack: &mut std::collections::HashSet<PackageId>,
    ) -> bool {
        visited.insert(node_id.clone());
        rec_stack.insert(node_id.clone());

        if let Some(dependencies) = self.edges.get(node_id) {
            for dep in dependencies {
                if !visited.contains(dep) && self.has_cycle_util(dep, visited, rec_stack) {
                    return true;
                }
                if rec_stack.contains(dep) {
                    return true;
                }
            }
        }

        rec_stack.remove(node_id);
        false
    }

    /// Perform topological sort using Kahn's algorithm
    pub fn topological_sort(&self) -> Result<Vec<PackageId>, spsv2_errors::Error> {
        use std::collections::{HashMap, VecDeque};

        if self.has_cycles() {
            return Err(spsv2_errors::PackageError::DependencyCycle {
                package: "unknown".to_string(),
            }
            .into());
        }

        // Calculate in-degrees
        let mut in_degree: HashMap<PackageId, usize> = HashMap::new();
        for node_id in self.nodes.keys() {
            in_degree.insert(node_id.clone(), 0);
        }

        for dependencies in self.edges.values() {
            for dep in dependencies {
                *in_degree.entry(dep.clone()).or_insert(0) += 1;
            }
        }

        // Find nodes with no incoming edges
        let mut queue: VecDeque<PackageId> = in_degree
            .iter()
            .filter_map(|(id, &degree)| if degree == 0 { Some(id.clone()) } else { None })
            .collect();

        let mut result = Vec::new();

        while let Some(node_id) = queue.pop_front() {
            result.push(node_id.clone());

            if let Some(dependencies) = self.edges.get(&node_id) {
                for dep in dependencies {
                    if let Some(degree) = in_degree.get_mut(dep) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dep.clone());
                        }
                    }
                }
            }
        }

        if result.len() != self.nodes.len() {
            return Err(spsv2_errors::PackageError::DependencyCycle {
                package: "unknown".to_string(),
            }
            .into());
        }

        Ok(result)
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spsv2_types::{Version, VersionSpec};

    #[test]
    fn test_package_id() {
        let id = PackageId::new("test".to_string(), Version::parse("1.0.0").unwrap());
        assert_eq!(id.to_string(), "test-1.0.0");
    }

    #[test]
    fn test_dep_edge() {
        let spec = VersionSpec::parse(">=1.0.0").unwrap();
        let edge = DepEdge::new("test".to_string(), spec, DepKind::Runtime);

        assert!(edge.satisfies(&Version::parse("1.0.0").unwrap()));
        assert!(edge.satisfies(&Version::parse("1.5.0").unwrap()));
        assert!(!edge.satisfies(&Version::parse("0.9.0").unwrap()));
    }

    #[test]
    fn test_resolved_node() {
        let deps = vec![
            DepEdge::new(
                "dep1".to_string(),
                VersionSpec::parse(">=1.0.0").unwrap(),
                DepKind::Runtime,
            ),
            DepEdge::new(
                "dep2".to_string(),
                VersionSpec::parse(">=2.0.0").unwrap(),
                DepKind::Build,
            ),
        ];

        let node = ResolvedNode::download(
            "test".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://example.com/test.sp".to_string(),
            deps,
        );

        assert_eq!(node.name, "test");
        assert_eq!(node.action, NodeAction::Download);
        assert_eq!(node.runtime_deps().count(), 1);
        assert_eq!(node.build_deps().count(), 1);
    }

    #[test]
    fn test_graph_cycle_detection() {
        let mut graph = DependencyGraph::new();

        // Add nodes
        let node_a = ResolvedNode::download(
            "a".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://example.com/a.sp".to_string(),
            vec![],
        );
        let node_b = ResolvedNode::download(
            "b".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://example.com/b.sp".to_string(),
            vec![],
        );

        let id_a = node_a.package_id();
        let id_b = node_b.package_id();

        graph.add_node(node_a);
        graph.add_node(node_b);

        // No cycles yet
        assert!(!graph.has_cycles());

        // Add cycle: a -> b -> a
        graph.add_edge(&id_a, &id_b);
        graph.add_edge(&id_b, &id_a);

        assert!(graph.has_cycles());
    }

    #[test]
    fn test_topological_sort() {
        let mut graph = DependencyGraph::new();

        // Create: a -> b -> c
        let node_a = ResolvedNode::download(
            "a".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://example.com/a.sp".to_string(),
            vec![],
        );
        let node_b = ResolvedNode::download(
            "b".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://example.com/b.sp".to_string(),
            vec![],
        );
        let node_c = ResolvedNode::download(
            "c".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://example.com/c.sp".to_string(),
            vec![],
        );

        let id_a = node_a.package_id();
        let id_b = node_b.package_id();
        let id_c = node_c.package_id();

        graph.add_node(node_a);
        graph.add_node(node_b);
        graph.add_node(node_c);

        // a depends on b, b depends on c
        graph.add_edge(&id_a, &id_b);
        graph.add_edge(&id_b, &id_c);

        let sorted = graph.topological_sort().unwrap();

        // c should come before b, b should come before a
        let pos_a = sorted.iter().position(|id| id == &id_a).unwrap();
        let pos_b = sorted.iter().position(|id| id == &id_b).unwrap();
        let pos_c = sorted.iter().position(|id| id == &id_c).unwrap();

        assert!(pos_c < pos_b);
        assert!(pos_b < pos_a);
    }
}
