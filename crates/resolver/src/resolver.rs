//! Main dependency resolver implementation

use crate::graph::DependencyGraph;
use crate::sat::{Clause, DependencyProblem, Literal, PackageVersion};
use crate::{
    DepEdge, DepKind, ExecutionPlan, PackageId, ResolutionContext, ResolutionResult, ResolvedNode,
};
use semver::Version;
use sps2_errors::{Error, PackageError};
use sps2_index::{IndexManager, VersionEntry};
use sps2_manifest::Manifest;
use sps2_types::package::PackageSpec;
use sps2_types::version::VersionConstraint;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Type alias for version entries map to reduce complexity
type VersionEntriesMap<'a> = HashMap<(String, Version), (&'a VersionEntry, DepKind)>;

/// Parameters for processing a single transitive dependency
#[allow(clippy::too_many_arguments)] // This is a parameter struct to reduce arguments
struct TransitiveDependencyParams<'a> {
    parent_name: &'a str,
    parent_version: &'a Version,
    dep_spec: &'a PackageSpec,
    dep_kind: DepKind,
}

/// Dependency resolver
#[derive(Clone, Debug)]
pub struct Resolver {
    /// Package index manager
    index: IndexManager,
}

impl Resolver {
    /// Create new resolver with index manager
    #[must_use]
    pub fn new(index: IndexManager) -> Self {
        Self { index }
    }

    /// Resolve dependencies for the given context
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A package is not found in the index
    /// - There are dependency cycles in the resolution graph
    /// - Version parsing fails
    /// - Package specifications are invalid
    pub async fn resolve(&self, context: ResolutionContext) -> Result<ResolutionResult, Error> {
        use tokio::time::{timeout, Duration};

        // Add overall timeout for dependency resolution
        let resolution_timeout = Duration::from_secs(120); // 2 minutes

        timeout(resolution_timeout, async {
            let mut graph = DependencyGraph::new();
            let mut visited = HashSet::new();

            // Process runtime dependencies
            for spec in &context.runtime_deps {
                self.resolve_package(spec, DepKind::Runtime, &mut graph, &mut visited)
                    .await?;
            }

            // Process build dependencies
            for spec in &context.build_deps {
                self.resolve_package(spec, DepKind::Build, &mut graph, &mut visited)
                    .await?;
            }

            // Process local files
            for path in &context.local_files {
                Self::resolve_local_file(path, &mut graph).await?;
            }

            // Check for cycles
            if graph.has_cycles() {
                return Err(PackageError::DependencyCycle {
                    package: "unknown".to_string(),
                }
                .into());
            }

            // Create execution plan
            let sorted = graph.topological_sort()?;
            let execution_plan = ExecutionPlan::from_sorted_packages(&sorted, &graph);

            Ok(ResolutionResult {
                nodes: graph.nodes,
                execution_plan,
            })
        })
        .await
        .map_err(|_| PackageError::ResolutionTimeout {
            message: "Dependency resolution timed out after 2 minutes".to_string(),
        })?
    }

    /// Resolve dependencies using SAT solver for more accurate resolution
    ///
    /// This method converts the dependency problem to a SAT problem and uses
    /// a DPLL-based solver with conflict-driven clause learning to find
    /// an optimal solution.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A package is not found in the index
    /// - No valid solution exists (conflicting constraints)
    /// - Version parsing fails
    pub async fn resolve_with_sat(
        &self,
        context: ResolutionContext,
    ) -> Result<ResolutionResult, Error> {
        use tokio::time::{timeout, Duration};

        let resolution_timeout = Duration::from_secs(120);

        timeout(resolution_timeout, async {
            // Create SAT problem and collect dependencies
            let (mut problem, package_deps) = Self::create_sat_problem(&context);

            // Add available versions and constraints
            let mut version_entries =
                self.add_package_versions_to_problem(&mut problem, &package_deps);

            // Process transitive dependencies
            self.process_transitive_dependencies(&mut problem, &mut version_entries);

            // Solve and convert to dependency graph
            let solution = crate::sat::solve_dependencies(problem).await?;
            let mut graph =
                Self::create_dependency_graph_from_solution(&solution, &version_entries)?;

            // Handle local files
            for path in &context.local_files {
                Self::resolve_local_file(path, &mut graph).await?;
            }

            // Create execution plan
            let sorted = graph.topological_sort()?;
            let execution_plan = ExecutionPlan::from_sorted_packages(&sorted, &graph);

            Ok(ResolutionResult {
                nodes: graph.nodes,
                execution_plan,
            })
        })
        .await
        .map_err(|_| PackageError::ResolutionTimeout {
            message: "SAT-based dependency resolution timed out after 2 minutes".to_string(),
        })?
    }

    /// Create SAT problem and collect dependency specifications
    fn create_sat_problem(
        context: &ResolutionContext,
    ) -> (
        DependencyProblem,
        HashMap<String, Vec<(PackageSpec, DepKind)>>,
    ) {
        let mut problem = DependencyProblem::new();
        let mut package_deps: HashMap<String, Vec<(PackageSpec, DepKind)>> = HashMap::new();

        // Collect all package specifications
        for spec in &context.runtime_deps {
            package_deps
                .entry(spec.name.clone())
                .or_default()
                .push((spec.clone(), DepKind::Runtime));
            problem.require_package(spec.name.clone());
        }

        for spec in &context.build_deps {
            package_deps
                .entry(spec.name.clone())
                .or_default()
                .push((spec.clone(), DepKind::Build));
            problem.require_package(spec.name.clone());
        }

        (problem, package_deps)
    }

    /// Add available package versions to the SAT problem
    fn add_package_versions_to_problem(
        &self,
        problem: &mut DependencyProblem,
        package_deps: &HashMap<String, Vec<(PackageSpec, DepKind)>>,
    ) -> VersionEntriesMap<'_> {
        let mut version_entries: HashMap<(String, Version), (&VersionEntry, DepKind)> =
            HashMap::new();

        for (package_name, specs) in package_deps {
            if let Some(index) = self.index.index() {
                if let Some(package_info) = index.packages.get(package_name) {
                    for (version_str, version_entry) in &package_info.versions {
                        if let Ok(version) = Version::parse(version_str) {
                            // Check if this version satisfies any of the specs
                            let mut satisfies_any = false;
                            let mut dep_kind = DepKind::Runtime;

                            for (spec, kind) in specs {
                                if spec.version_spec.matches(&version) {
                                    satisfies_any = true;
                                    dep_kind = *kind;
                                    break;
                                }
                            }

                            if satisfies_any {
                                let pv = PackageVersion::new(package_name.clone(), version.clone());
                                problem.add_package_version(pv);
                                version_entries.insert(
                                    (package_name.clone(), version),
                                    (version_entry, dep_kind),
                                );
                            }
                        }
                    }
                }
            }

            // Add constraints for each package specification
            // At most one version can be selected
            problem.add_at_most_one_constraint(package_name);

            // At least one version must be selected (for required packages)
            problem.add_at_least_one_constraint(package_name);

            // Add version constraints as clauses
            for (spec, _kind) in specs {
                Self::add_version_constraints(problem, spec);
            }
        }

        version_entries
    }

    /// Process transitive dependencies
    fn process_transitive_dependencies<'a>(
        &'a self,
        problem: &mut DependencyProblem,
        version_entries: &mut VersionEntriesMap<'a>,
    ) {
        let mut processed = HashSet::new();
        let mut to_process: Vec<(String, Version, DepKind)> = Vec::new();

        // Initialize with direct dependencies
        for ((name, version), (_entry, kind)) in &*version_entries {
            to_process.push((name.clone(), version.clone(), *kind));
        }

        while let Some((pkg_name, pkg_version, parent_kind)) = to_process.pop() {
            let key = (pkg_name.clone(), pkg_version.clone());
            if processed.contains(&key) {
                continue;
            }
            processed.insert(key.clone());

            // Clone the dependencies we need to process
            let deps_to_process = if let Some((version_entry, _)) = version_entries.get(&key) {
                let mut deps = Vec::new();

                // Collect runtime dependencies
                for dep_str in &version_entry.dependencies.runtime {
                    if let Ok(dep_spec) = PackageSpec::parse(dep_str) {
                        deps.push((dep_spec, DepKind::Runtime));
                    }
                }

                // Collect build dependencies if this is a build dependency
                if parent_kind == DepKind::Build {
                    for dep_str in &version_entry.dependencies.build {
                        if let Ok(dep_spec) = PackageSpec::parse(dep_str) {
                            deps.push((dep_spec, DepKind::Build));
                        }
                    }
                }

                deps
            } else {
                Vec::new()
            };

            // Now process the dependencies - process each one separately to avoid borrow issues
            for (dep_spec, dep_kind) in deps_to_process {
                let params = TransitiveDependencyParams {
                    parent_name: &pkg_name,
                    parent_version: &pkg_version,
                    dep_spec: &dep_spec,
                    dep_kind,
                };
                self.process_single_transitive_dependency(
                    problem,
                    &mut to_process,
                    version_entries,
                    &params,
                );
            }
        }
    }

    /// Create dependency graph from SAT solution
    fn create_dependency_graph_from_solution(
        solution: &crate::sat::DependencySolution,
        version_entries: &VersionEntriesMap<'_>,
    ) -> Result<DependencyGraph, Error> {
        let mut graph = DependencyGraph::new();
        let mut resolved_nodes = HashMap::new();

        // Create nodes for selected packages
        for (name, version) in &solution.selected {
            let key = (name.clone(), version.clone());
            if let Some((version_entry, _kind)) = version_entries.get(&key) {
                let package_id = PackageId::new(name.clone(), version.clone());

                // Create dependency edges
                let mut deps = Vec::new();
                for dep_str in &version_entry.dependencies.runtime {
                    if let Ok(dep_spec) = PackageSpec::parse(dep_str) {
                        deps.push(DepEdge::new(
                            dep_spec.name.clone(),
                            dep_spec.version_spec,
                            DepKind::Runtime,
                        ));
                    }
                }

                let node = ResolvedNode::download(
                    name.clone(),
                    version.clone(),
                    Self::resolve_download_url(&version_entry.download_url)?,
                    deps,
                );

                resolved_nodes.insert(package_id.clone(), node.clone());
                graph.add_node(node);
            }
        }

        // Add edges to graph
        for (package_id, node) in &resolved_nodes {
            for edge in &node.deps {
                // Find the resolved version of the dependency
                if let Some(dep_version) = solution.selected.get(&edge.name) {
                    let dep_id = PackageId::new(edge.name.clone(), dep_version.clone());
                    graph.add_edge(&dep_id, package_id);
                }
            }
        }

        Ok(graph)
    }

    /// Add version constraints to SAT problem
    fn add_version_constraints(problem: &mut DependencyProblem, spec: &PackageSpec) {
        // Clone the versions to avoid borrow issues
        let versions = problem
            .get_package_versions(&spec.name)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();

        for constraint in spec.version_spec.constraints() {
            match constraint {
                VersionConstraint::Exact(v) => {
                    // Only the exact version can be true
                    for pv in &versions {
                        if &pv.version != v {
                            if let Some(var) = problem.variables.get_variable(pv) {
                                problem.add_clause(Clause::unit(Literal::negative(var)));
                            }
                        }
                    }
                }
                VersionConstraint::NotEqual(v) => {
                    // The specified version must be false
                    for pv in &versions {
                        if &pv.version == v {
                            if let Some(var) = problem.variables.get_variable(pv) {
                                problem.add_clause(Clause::unit(Literal::negative(var)));
                            }
                        }
                    }
                }
                _ => {
                    // For range constraints, we rely on version filtering during problem setup
                }
            }
        }
    }

    /// Process a single transitive dependency
    fn process_single_transitive_dependency<'a>(
        &'a self,
        problem: &mut DependencyProblem,
        to_process: &mut Vec<(String, Version, DepKind)>,
        version_entries: &mut VersionEntriesMap<'a>,
        params: &TransitiveDependencyParams<'_>,
    ) {
        let parent_pv = PackageVersion::new(
            params.parent_name.to_string(),
            params.parent_version.clone(),
        );

        if let Some(index) = self.index.index() {
            if let Some(package_info) = index.packages.get(&params.dep_spec.name) {
                let mut valid_versions = Vec::new();

                for (version_str, version_entry) in &package_info.versions {
                    if let Ok(version) = Version::parse(version_str) {
                        if params.dep_spec.version_spec.matches(&version) {
                            let dep_pv =
                                PackageVersion::new(params.dep_spec.name.clone(), version.clone());
                            let dep_var = problem.add_package_version(dep_pv);
                            valid_versions.push(dep_var);

                            // Add to version entries
                            version_entries.insert(
                                (params.dep_spec.name.clone(), version.clone()),
                                (version_entry, params.dep_kind),
                            );

                            // Add to processing queue
                            to_process.push((
                                params.dep_spec.name.clone(),
                                version,
                                params.dep_kind,
                            ));
                        }
                    }
                }

                if !valid_versions.is_empty() {
                    // Add implication: parent => (dep1 OR dep2 OR ...)
                    // Which is equivalent to: !parent OR dep1 OR dep2 OR ...
                    if let Some(parent_var) = problem.variables.get_variable(&parent_pv) {
                        let mut clause_lits = vec![Literal::negative(parent_var)];
                        clause_lits.extend(valid_versions.into_iter().map(Literal::positive));
                        problem.add_clause(Clause::new(clause_lits));
                    }

                    // Ensure at most one version of the dependency
                    problem.add_at_most_one_constraint(&params.dep_spec.name);
                }
            }
        }
    }

    /// Resolve a single package and its dependencies with depth limiting
    async fn resolve_package(
        &self,
        spec: &PackageSpec,
        dep_kind: DepKind,
        graph: &mut DependencyGraph,
        visited: &mut HashSet<PackageId>,
    ) -> Result<(), Error> {
        self.resolve_package_with_depth(spec, dep_kind, graph, visited, 0)
            .await
    }

    /// Resolve a single package and its dependencies with recursion depth limit
    async fn resolve_package_with_depth(
        &self,
        spec: &PackageSpec,
        dep_kind: DepKind,
        graph: &mut DependencyGraph,
        visited: &mut HashSet<PackageId>,
        depth: usize,
    ) -> Result<(), Error> {
        const MAX_RECURSION_DEPTH: usize = 100;

        if depth > MAX_RECURSION_DEPTH {
            return Err(PackageError::DependencyCycle {
                package: format!("depth limit exceeded for {}", spec.name),
            }
            .into());
        }
        // Find best version matching the spec
        let (version_str, version_entry) = self
            .index
            .find_best_version_with_string(spec)
            .ok_or_else(|| PackageError::NotFound {
                name: spec.name.clone(),
            })?;

        let version = Version::parse(version_str)?;
        let package_id = PackageId::new(spec.name.clone(), version.clone());

        // Skip if already processed
        if visited.contains(&package_id) {
            return Ok(());
        }
        visited.insert(package_id.clone());

        // Create dependency edges
        let mut deps = Vec::new();

        // Add runtime dependencies
        for dep_spec_str in &version_entry.dependencies.runtime {
            let dep_spec = PackageSpec::parse(dep_spec_str)?;
            let edge = DepEdge::new(
                dep_spec.name.clone(),
                dep_spec.version_spec.clone(),
                DepKind::Runtime,
            );
            deps.push(edge);
        }

        // Add build dependencies (only if this is a build dependency)
        if dep_kind == DepKind::Build {
            for dep_spec_str in &version_entry.dependencies.build {
                let dep_spec = PackageSpec::parse(dep_spec_str)?;
                let edge = DepEdge::new(
                    dep_spec.name.clone(),
                    dep_spec.version_spec.clone(),
                    DepKind::Build,
                );
                deps.push(edge);
            }
        }

        // Create resolved node with URL resolution
        let node = ResolvedNode::download(
            spec.name.clone(),
            version,
            Self::resolve_download_url(&version_entry.download_url)?,
            deps.clone(),
        );

        graph.add_node(node);

        // Recursively resolve dependencies
        for edge in &deps {
            let dep_spec = PackageSpec {
                name: edge.name.clone(),
                version_spec: edge.spec.clone(),
            };

            Box::pin(self.resolve_package_with_depth(
                &dep_spec,
                edge.kind,
                graph,
                visited,
                depth + 1,
            ))
            .await?;

            // Find resolved dependency in graph and add edge (FIXED: single resolution)
            if let Some(dep_node) = graph.nodes.values().find(|n| n.name == edge.name) {
                let dep_id = dep_node.package_id();
                // Edge direction: dep_id -> package_id (dependency points to dependent)
                // If curl depends on openssl, edge is from openssl to curl
                graph.add_edge(&dep_id, &package_id);
            }

            // Check for cycles after each dependency addition
            if graph.has_cycles() {
                return Err(PackageError::DependencyCycle {
                    package: package_id.name.clone(),
                }
                .into());
            }
        }

        Ok(())
    }

    /// Resolve a local package file
    async fn resolve_local_file(path: &Path, graph: &mut DependencyGraph) -> Result<(), Error> {
        // Load manifest from local .sp file
        let manifest = Self::load_local_manifest(path).await?;

        let version = Version::parse(&manifest.package.version)?;
        let _package_id = PackageId::new(manifest.package.name.clone(), version.clone());

        // Create dependency edges from manifest
        let mut deps = Vec::new();

        for dep in &manifest.dependencies.runtime {
            // Parse dependency spec
            let dep_spec = PackageSpec::parse(dep)?;
            let edge = DepEdge::new(
                dep_spec.name.clone(),
                dep_spec.version_spec,
                DepKind::Runtime,
            );
            deps.push(edge);
        }

        // Create resolved node for local file
        let node = ResolvedNode::local(manifest.package.name, version, path.to_path_buf(), deps);

        graph.add_node(node);

        Ok(())
    }

    /// Load manifest from local .sp file
    async fn load_local_manifest(path: &Path) -> Result<Manifest, Error> {
        use tokio::fs;
        use tokio::process::Command;

        // Create temporary directory for extraction
        let temp_dir =
            std::env::temp_dir().join(format!("sps2_manifest_{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&temp_dir).await?;

        // Ensure cleanup on error
        let _cleanup_guard = scopeguard::guard(&temp_dir, |temp_dir| {
            if temp_dir.exists() {
                let _ = std::fs::remove_dir_all(temp_dir);
            }
        });

        // Step 1: Decompress .sp file with zstd to get tar file
        let tar_path = temp_dir.join("package.tar");
        let zstd_output = Command::new("zstd")
            .args([
                "--decompress",
                "-o",
                &tar_path.display().to_string(),
                &path.display().to_string(),
            ])
            .output()
            .await?;

        if !zstd_output.status.success() {
            return Err(PackageError::InvalidFormat {
                message: format!(
                    "failed to decompress .sp file: {}",
                    String::from_utf8_lossy(&zstd_output.stderr)
                ),
            }
            .into());
        }

        // Step 2: Extract only manifest.toml from tar archive
        let manifest_content = Self::extract_manifest_from_tar(&tar_path).await?;

        // Step 3: Parse the manifest
        let manifest = Manifest::from_toml(&manifest_content)?;

        Ok(manifest)
    }

    /// Extract manifest.toml content from tar archive
    async fn extract_manifest_from_tar(tar_path: &Path) -> Result<String, Error> {
        use tokio::process::Command;

        // Use tar to extract just the manifest.toml file and output to stdout
        let tar_output = Command::new("tar")
            .args([
                "--extract",
                "--file",
                &tar_path.display().to_string(),
                "--to-stdout",
                "manifest.toml",
            ])
            .output()
            .await?;

        if !tar_output.status.success() {
            return Err(PackageError::InvalidFormat {
                message: format!(
                    "failed to extract manifest from tar: {}",
                    String::from_utf8_lossy(&tar_output.stderr)
                ),
            }
            .into());
        }

        let content =
            String::from_utf8(tar_output.stdout).map_err(|_| PackageError::InvalidFormat {
                message: "manifest.toml contains invalid UTF-8".to_string(),
            })?;

        if content.trim().is_empty() {
            return Err(PackageError::InvalidFormat {
                message: "manifest.toml is empty or missing".to_string(),
            }
            .into());
        }

        Ok(content)
    }

    /// Get available versions for a package
    #[must_use]
    pub fn get_package_versions(&self, name: &str) -> Option<Vec<&VersionEntry>> {
        self.index.get_package_versions(name)
    }

    /// Search for packages
    #[must_use]
    pub fn search_packages(&self, query: &str) -> Vec<&str> {
        self.index.search(query)
    }

    /// Check if a package exists
    #[must_use]
    pub fn package_exists(&self, name: &str) -> bool {
        self.index.get_package_versions(name).is_some()
    }

    /// Find best version for a package spec
    #[must_use]
    pub fn find_best_version(&self, spec: &PackageSpec) -> Option<&VersionEntry> {
        self.index.find_best_version(spec)
    }

    /// Resolve download URL with repository integration
    ///
    /// This is currently a pass-through but will be enhanced for:
    /// - Mirror failover
    /// - CDN optimization
    /// - Repository URL resolution
    fn resolve_download_url(url: &str) -> Result<String, Error> {
        // For now, pass through the URL as-is
        // Future enhancements:
        // - Check for repository URL patterns and resolve to CDN
        // - Support mirror failover
        // - Handle repository index entries

        // Basic URL validation
        if url.is_empty() {
            return Err(PackageError::InvalidFormat {
                message: "empty download URL".to_string(),
            }
            .into());
        }

        // Ensure HTTPS for security (skip in test mode or when explicitly disabled)
        // Allow HTTP in test environments
        let allow_http = std::env::var("SPS2_ALLOW_HTTP").is_ok();

        if !allow_http && url.starts_with("http://") {
            return Ok(url.replace("http://", "https://"));
        }

        Ok(url.to_string())
    }
}

/// Resolution constraints for builds vs installs
#[derive(Clone, Debug)]
#[allow(dead_code)] // Designed for future use when build/install logic is enhanced
pub struct ResolutionConstraints {
    /// Include build dependencies
    pub include_build_deps: bool,
    /// Maximum resolution depth
    pub max_depth: Option<usize>,
    /// Allowed architectures
    pub allowed_archs: Vec<String>,
}

impl Default for ResolutionConstraints {
    fn default() -> Self {
        Self {
            include_build_deps: false,
            max_depth: None,
            allowed_archs: vec!["arm64".to_string()],
        }
    }
}

impl ResolutionConstraints {
    /// Create constraints for installation (runtime deps only)
    #[allow(dead_code)] // Will be used when installer distinguishes build vs runtime deps
    pub fn for_install() -> Self {
        Self {
            include_build_deps: false,
            ..Default::default()
        }
    }

    /// Create constraints for building (include build deps)
    #[allow(dead_code)] // Will be used when builder needs to resolve build dependencies
    pub fn for_build() -> Self {
        Self {
            include_build_deps: true,
            ..Default::default()
        }
    }
}
