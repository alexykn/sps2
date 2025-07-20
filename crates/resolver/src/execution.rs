// src/execution.rs
//! Planning and metadata for *parallel* package installation/build.
//!
//! Public API is **unchanged**, but the internals are optimised

use crate::{graph::DependencyGraph, NodeAction, PackageId};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Per-node execution metadata.
///
/// The struct is `Arc`-wrapped inside the [`ExecutionPlan`]; cloning the `Arc`
/// is cheap and thread-safe.
#[derive(Debug)]
pub struct NodeMeta {
    /// Action that the installer/runner must perform.
    pub action: NodeAction,
    /// *Remaining* unsatisfied dependencies.
    /// Once this reaches 0 the package is runnable.
    in_degree: AtomicUsize,
    /// Packages that depend on this one (reverse edges).
    ///
    /// This field is kept `pub` for backwards-compatibility with the
    /// `sps2-install` crate.  New code should prefer the
    /// [`ExecutionPlan::complete_package`] API.
    pub parents: Vec<PackageId>,
}

impl NodeMeta {
    /// New metadata with a fixed initial in-degree.
    #[inline]
    #[must_use]
    pub fn new(action: NodeAction, in_degree: usize) -> Self {
        Self {
            action,
            in_degree: AtomicUsize::new(in_degree),
            parents: Vec::new(),
        }
    }

    /// Thread-safe decrement; returns the **updated** in-degree (never under-flows).
    ///
    /// If the counter is already 0 the call is a no-op and 0 is returned.
    #[inline]
    #[must_use]
    pub fn decrement_in_degree(&self) -> usize {
        // `fetch_update` loops internally until CAS succeeds.
        self.in_degree
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current != 0).then(|| current - 1)
            })
            .map(|prev| prev.saturating_sub(1))
            .unwrap_or(0)
    }

    /// Current unsatisfied dependency count.
    #[inline]
    #[must_use]
    pub fn in_degree(&self) -> usize {
        self.in_degree.load(Ordering::Acquire)
    }

    /// Register `parent` as a reverse-edge.
    #[inline]
    pub fn add_parent(&mut self, parent: PackageId) {
        self.parents.push(parent);
    }

    /// Immutable view of reverse-edges.
    #[inline]
    #[must_use]
    pub fn parents(&self) -> &[PackageId] {
        &self.parents
    }
}

/// Immutable execution plan – produced **once** after resolution
/// and consumed concurrently by the installer/runner.
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    batches: Vec<Vec<PackageId>>,
    metadata: HashMap<PackageId, Arc<NodeMeta>>,
}

impl ExecutionPlan {
    // ---------------------------------------------------------------------
    // Construction
    // ---------------------------------------------------------------------

    /// Build a plan from an already topologically-sorted list (`sorted`) and
    /// its originating dependency graph.
    ///
    /// # Panics
    ///
    /// Panics if `graph` does not contain every [`PackageId`] present in
    /// `sorted` (the resolver guarantees this invariant).
    #[must_use]
    pub fn from_sorted_packages(sorted: &[PackageId], graph: &DependencyGraph) -> Self {
        let mut metadata: HashMap<PackageId, Arc<NodeMeta>> = HashMap::with_capacity(sorted.len());
        let mut in_degree: HashMap<&PackageId, usize> = HashMap::with_capacity(sorted.len());

        // 1) Pre-compute in-degrees in O(e)
        for id in sorted {
            in_degree.insert(id, 0);
        }
        for tos in graph.edges.values() {
            for to in tos {
                if let Some(slot) = in_degree.get_mut(to) {
                    *slot += 1;
                }
            }
        }

        // 2) Create NodeMeta and reverse edges
        for id in sorted {
            let action = graph
                .nodes
                .get(id)
                .map(|n| n.action.clone())
                .expect("package missing from graph");

            let meta = Arc::new(NodeMeta::new(
                action,
                *in_degree.get(id).expect("key present"),
            ));
            metadata.insert(id.clone(), meta);
        }

        for (from, tos) in &graph.edges {
            for to in tos {
                if let Some(meta) = metadata.get_mut(from).and_then(Arc::get_mut) {
                    meta.add_parent(to.clone());
                }
            }
        }

        // 3) Kahn layering to build parallel batches in O(n + e)
        let mut queue: VecDeque<&PackageId> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(id, _)| *id)
            .collect();

        let mut batches: Vec<Vec<PackageId>> = Vec::new();
        let mut remaining = in_degree.len();

        while remaining > 0 {
            let mut batch: Vec<PackageId> = Vec::with_capacity(queue.len());

            for _ in 0..queue.len() {
                let id = queue.pop_front().expect("queue not empty");
                batch.push(id.clone());
                remaining -= 1;

                // Decrement children
                if let Some(children) = graph.edges.get(id) {
                    for child in children {
                        let child_meta = metadata
                            .get(child)
                            .expect("child in metadata; resolver invariant");
                        if child_meta.decrement_in_degree() == 0 {
                            queue.push_back(child);
                        }
                    }
                }
            }

            batches.push(batch);
        }

        Self { batches, metadata }
    }

    // ---------------------------------------------------------------------
    // Inspection helpers
    // ---------------------------------------------------------------------

    /// Layered batches; inside each slice packages are independent.
    #[inline]
    #[must_use]
    pub fn batches(&self) -> &[Vec<PackageId>] {
        &self.batches
    }

    /// Per-package metadata (constant during execution).
    #[inline]
    #[must_use]
    pub fn metadata(&self, id: &PackageId) -> Option<&Arc<NodeMeta>> {
        self.metadata.get(id)
    }

    /// All packages whose `in_degree == 0` **at plan creation time**.
    #[inline]
    #[must_use]
    pub fn initial_ready(&self) -> Vec<PackageId> {
        self.metadata
            .iter()
            .filter(|(_, m)| m.in_degree() == 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Legacy alias used by the installer: forwards to [`Self::initial_ready`].
    #[inline]
    #[must_use]
    pub fn ready_packages(&self) -> Vec<PackageId> {
        self.initial_ready()
    }

    /// Mark `finished` as completed and return **newly** unblocked packages.
    ///
    /// # Panics
    ///
    /// Panics if
    /// 1. `finished` is unknown to this [`ExecutionPlan`] (violates resolver
    ///    invariant), **or**
    /// 2. any parent package listed in `NodeMeta::parents` cannot be found in
    ///    the plan’s metadata map (also a resolver invariant).
    #[inline]
    #[must_use]
    pub fn complete_package(&self, finished: &PackageId) -> Vec<PackageId> {
        let meta = self
            .metadata
            .get(finished)
            .expect("completed package known to plan");

        meta.parents
            .iter()
            .filter_map(|parent| {
                let parent_meta = self
                    .metadata
                    .get(parent)
                    .expect("parent package known to plan");
                (parent_meta.decrement_in_degree() == 0).then(|| parent.clone())
            })
            .collect()
    }

    // ---------------------------------------------------------------------
    // Convenience metrics
    // ---------------------------------------------------------------------

    #[inline]
    #[must_use]
    pub fn package_count(&self) -> usize {
        self.metadata.len()
    }

    #[inline]
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.metadata.values().all(|m| m.in_degree() == 0)
    }

    #[inline]
    #[must_use]
    pub fn completed_count(&self) -> usize {
        self.metadata
            .values()
            .filter(|m| m.in_degree() == 0)
            .count()
    }
}

// -------------------------------------------------------------------------
// Stats helper (unchanged public fields, lint-clean implementation)
// -------------------------------------------------------------------------
