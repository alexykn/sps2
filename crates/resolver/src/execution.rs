//! Execution plan for parallel dependency installation

use crate::{NodeAction, PackageId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Metadata for parallel execution
#[derive(Debug)]
pub struct NodeMeta {
    /// Action to perform
    pub action: NodeAction,
    /// Number of unresolved dependencies
    pub in_degree: AtomicUsize,
    /// Packages that depend on this one
    pub parents: Vec<PackageId>,
}

impl NodeMeta {
    /// Create new node metadata
    pub fn new(action: NodeAction, in_degree: usize) -> Self {
        Self {
            action,
            in_degree: AtomicUsize::new(in_degree),
            parents: Vec::new(),
        }
    }

    /// Decrement in-degree and return new value
    pub fn decrement_in_degree(&self) -> usize {
        self.in_degree
            .fetch_sub(1, Ordering::SeqCst)
            .saturating_sub(1)
    }

    /// Get current in-degree
    pub fn in_degree(&self) -> usize {
        self.in_degree.load(Ordering::SeqCst)
    }

    /// Add parent dependency
    pub fn add_parent(&mut self, parent: PackageId) {
        self.parents.push(parent);
    }
}

/// Execution plan with batched parallel operations
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    /// Execution batches (can run in parallel within each batch)
    batches: Vec<Vec<PackageId>>,
    /// Metadata for parallel execution
    metadata: HashMap<PackageId, Arc<NodeMeta>>,
}

impl ExecutionPlan {
    /// Create execution plan from topologically sorted packages
    pub fn from_sorted_packages(
        sorted: Vec<PackageId>,
        graph: &crate::graph::DependencyGraph,
    ) -> Self {
        let mut metadata = HashMap::new();
        let mut batches = Vec::new();
        let mut remaining: std::collections::HashSet<PackageId> = sorted.iter().cloned().collect();

        // Initialize metadata
        for package_id in &sorted {
            if let Some(node) = graph.nodes.get(package_id) {
                let in_degree = graph.edges.get(package_id).map_or(0, |deps| deps.len());

                let meta = Arc::new(NodeMeta::new(node.action.clone(), in_degree));
                metadata.insert(package_id.clone(), meta);
            }
        }

        // Build parent relationships
        for (package_id, dependencies) in &graph.edges {
            if let Some(meta) = metadata.get_mut(package_id) {
                for dep in dependencies {
                    if let Some(meta) = Arc::get_mut(meta) {
                        meta.add_parent(package_id.clone());
                    }
                }
            }
        }

        // Create batches by finding packages with no dependencies
        while !remaining.is_empty() {
            let mut batch = Vec::new();

            // Find packages with no unresolved dependencies
            for package_id in &remaining {
                let deps_count = graph.edges.get(package_id).map_or(0, |deps| {
                    deps.iter().filter(|dep| remaining.contains(dep)).count()
                });

                if deps_count == 0 {
                    batch.push(package_id.clone());
                }
            }

            if batch.is_empty() {
                // This shouldn't happen with a valid topological sort
                break;
            }

            // Remove batched packages from remaining
            for package_id in &batch {
                remaining.remove(package_id);
            }

            batches.push(batch);
        }

        Self { batches, metadata }
    }

    /// Get execution batches
    pub fn batches(&self) -> &[Vec<PackageId>] {
        &self.batches
    }

    /// Get metadata for a package
    pub fn metadata(&self, package_id: &PackageId) -> Option<&Arc<NodeMeta>> {
        self.metadata.get(package_id)
    }

    /// Get packages that are ready to execute (no dependencies)
    pub fn ready_packages(&self) -> Vec<PackageId> {
        self.metadata
            .iter()
            .filter_map(|(id, meta)| {
                if meta.in_degree() == 0 {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Mark package as completed and get newly ready packages
    pub fn complete_package(&self, package_id: &PackageId) -> Vec<PackageId> {
        let Some(meta) = self.metadata.get(package_id) else {
            return Vec::new();
        };

        let mut newly_ready = Vec::new();

        // Decrement in-degree for all parents
        for parent_id in &meta.parents {
            if let Some(parent_meta) = self.metadata.get(parent_id) {
                if parent_meta.decrement_in_degree() == 0 {
                    newly_ready.push(parent_id.clone());
                }
            }
        }

        newly_ready
    }

    /// Get total number of packages
    pub fn package_count(&self) -> usize {
        self.metadata.len()
    }

    /// Check if all packages are completed
    pub fn is_complete(&self) -> bool {
        self.metadata.values().all(|meta| meta.in_degree() == 0)
    }
}

/// Execution statistics
#[derive(Debug, Default)]
pub struct ExecutionStats {
    /// Total packages processed
    pub total_packages: usize,
    /// Packages downloaded
    pub downloaded: usize,
    /// Local packages used
    pub local: usize,
    /// Number of parallel batches
    pub batch_count: usize,
    /// Maximum batch size
    pub max_batch_size: usize,
}

impl ExecutionStats {
    /// Calculate stats from execution plan
    pub fn from_plan(plan: &ExecutionPlan, graph: &crate::graph::DependencyGraph) -> Self {
        let mut stats = Self {
            total_packages: plan.package_count(),
            batch_count: plan.batches().len(),
            max_batch_size: plan.batches().iter().map(Vec::len).max().unwrap_or(0),
            ..Default::default()
        };

        // Count action types
        for package_id in plan.metadata.keys() {
            if let Some(node) = graph.nodes.get(package_id) {
                match node.action {
                    NodeAction::Download => stats.downloaded += 1,
                    NodeAction::Local => stats.local += 1,
                }
            }
        }

        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{DependencyGraph, ResolvedNode};
    use spsv2_types::Version;

    #[test]
    fn test_node_meta() {
        let meta = NodeMeta::new(NodeAction::Download, 2);
        assert_eq!(meta.in_degree(), 2);

        assert_eq!(meta.decrement_in_degree(), 1);
        assert_eq!(meta.in_degree(), 1);

        assert_eq!(meta.decrement_in_degree(), 0);
        assert_eq!(meta.in_degree(), 0);
    }

    #[test]
    fn test_execution_plan() {
        let mut graph = DependencyGraph::new();

        // Create: a -> b -> c (c has no deps, b depends on c, a depends on b)
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
        let plan = ExecutionPlan::from_sorted_packages(sorted, &graph);

        // Should have 3 batches: [c], [b], [a]
        assert_eq!(plan.batches().len(), 3);
        assert_eq!(plan.batches()[0], vec![id_c.clone()]);
        assert_eq!(plan.batches()[1], vec![id_b.clone()]);
        assert_eq!(plan.batches()[2], vec![id_a.clone()]);

        // Initially only c should be ready
        let ready = plan.ready_packages();
        assert_eq!(ready, vec![id_c]);

        // After completing c, b should be ready
        let newly_ready = plan.complete_package(&id_c);
        assert_eq!(newly_ready, vec![id_b]);
    }

    #[test]
    fn test_execution_stats() {
        let mut graph = DependencyGraph::new();

        let node_a = ResolvedNode::download(
            "a".to_string(),
            Version::parse("1.0.0").unwrap(),
            "https://example.com/a.sp".to_string(),
            vec![],
        );
        let node_b = ResolvedNode::local(
            "b".to_string(),
            Version::parse("1.0.0").unwrap(),
            "/path/to/b.sp".into(),
            vec![],
        );

        graph.add_node(node_a);
        graph.add_node(node_b);

        let sorted = graph.topological_sort().unwrap();
        let plan = ExecutionPlan::from_sorted_packages(sorted, &graph);
        let stats = ExecutionStats::from_plan(&plan, &graph);

        assert_eq!(stats.total_packages, 2);
        assert_eq!(stats.downloaded, 1);
        assert_eq!(stats.local, 1);
    }
}
