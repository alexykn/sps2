//! Main dependency resolver implementation

use crate::{
    DepEdge, DepKind, ExecutionPlan, PackageId, ResolutionContext,
    ResolutionResult, ResolvedNode,
};
use crate::graph::DependencyGraph;
use spsv2_errors::{Error, PackageError};
use spsv2_index::{IndexManager, VersionEntry};
use spsv2_manifest::Manifest;
use spsv2_types::package::PackageSpec;
use spsv2_types::version::VersionSpec;
use semver::Version;
use std::str::FromStr;
use std::collections::HashSet;
use std::path::Path;

/// Dependency resolver
#[derive(Clone)]
pub struct Resolver {
    /// Package index manager
    index: IndexManager,
}

impl Resolver {
    /// Create new resolver with index manager
    pub fn new(index: IndexManager) -> Self {
        Self { index }
    }

    /// Resolve dependencies for the given context
    pub async fn resolve(&self, context: ResolutionContext) -> Result<ResolutionResult, Error> {
        let mut graph = DependencyGraph::new();
        let mut visited = HashSet::new();

        // Process runtime dependencies
        for spec in &context.runtime_deps {
            self.resolve_package(&spec, DepKind::Runtime, &mut graph, &mut visited)
                .await?;
        }

        // Process build dependencies
        for spec in &context.build_deps {
            self.resolve_package(&spec, DepKind::Build, &mut graph, &mut visited)
                .await?;
        }

        // Process local files
        for path in &context.local_files {
            self.resolve_local_file(path, &mut graph).await?;
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
        let execution_plan = ExecutionPlan::from_sorted_packages(sorted, &graph);

        Ok(ResolutionResult {
            nodes: graph.nodes,
            execution_plan,
        })
    }

    /// Resolve a single package and its dependencies
    async fn resolve_package(
        &self,
        spec: &PackageSpec,
        dep_kind: DepKind,
        graph: &mut DependencyGraph,
        visited: &mut HashSet<PackageId>,
    ) -> Result<(), Error> {
        // Find best version matching the spec
        let version_entry =
            self.index
                .find_best_version(spec)
                .ok_or_else(|| PackageError::NotFound {
                    name: spec.name.clone(),
                })?;

        let version = Version::parse(&version_entry.version())?;
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

        // Create resolved node
        let node = ResolvedNode::download(
            spec.name.clone(),
            version,
            version_entry.download_url.clone(),
            deps.clone(),
        );

        graph.add_node(node);

        // Recursively resolve dependencies
        for edge in &deps {
            let dep_spec = PackageSpec {
                name: edge.name.clone(),
                version_spec: edge.spec.clone(),
            };

            Box::pin(self.resolve_package(&dep_spec, edge.kind.clone(), graph, visited))
                .await?;

            // Add edge to graph
            let dep_version =
                self.index
                    .find_best_version(&dep_spec)
                    .ok_or_else(|| PackageError::NotFound {
                        name: dep_spec.name.clone(),
                    })?;

            let dep_version_parsed = Version::parse(&dep_version.version())?;
            let dep_id = PackageId::new(edge.name.clone(), dep_version_parsed);

            graph.add_edge(&package_id, &dep_id);
        }

        Ok(())
    }

    /// Resolve a local package file
    async fn resolve_local_file(
        &self,
        path: &Path,
        graph: &mut DependencyGraph,
    ) -> Result<(), Error> {
        // Load manifest from local .sp file
        let manifest = self.load_local_manifest(path).await?;

        let version = Version::parse(&manifest.package.version)?;
        let package_id = PackageId::new(manifest.package.name.clone(), version.clone());

        // Create dependency edges from manifest
        let mut deps = Vec::new();

        for dep in &manifest.dependencies.runtime {
            let spec = VersionSpec::from_str(dep)?;
            // Parse dependency spec
            let dep_spec = PackageSpec::parse(dep)?;
            let edge = DepEdge::new(dep_spec.name.clone(), dep_spec.version_spec, DepKind::Runtime);
            deps.push(edge);
        }

        // Create resolved node for local file
        let node = ResolvedNode::local(manifest.package.name, version, path.to_path_buf(), deps);

        graph.add_node(node);

        Ok(())
    }

    /// Load manifest from local .sp file
    async fn load_local_manifest(&self, path: &Path) -> Result<Manifest, Error> {
        // For now, just return an error - this would need integration with store crate
        // to extract and read the manifest from the .sp archive
        Err(PackageError::InvalidFormat {
            message: format!(
                "local package loading not yet implemented: {}",
                path.display()
            ),
        }
        .into())
    }

    /// Get available versions for a package
    pub fn get_package_versions(&self, name: &str) -> Option<Vec<&VersionEntry>> {
        self.index.get_package_versions(name)
    }

    /// Search for packages
    pub fn search_packages(&self, query: &str) -> Vec<&str> {
        self.index.search(query)
    }

    /// Check if a package exists
    pub fn package_exists(&self, name: &str) -> bool {
        self.index.get_package_versions(name).is_some()
    }

    /// Find best version for a package spec
    pub fn find_best_version(&self, spec: &PackageSpec) -> Option<&VersionEntry> {
        self.index.find_best_version(spec)
    }
}

/// Resolution constraints for builds vs installs
#[derive(Clone, Debug)]
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
    pub fn for_install() -> Self {
        Self {
            include_build_deps: false,
            ..Default::default()
        }
    }

    /// Create constraints for building (include build deps)
    pub fn for_build() -> Self {
        Self {
            include_build_deps: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spsv2_index::{DependencyInfo, Index, IndexManager, VersionEntry};
    use tempfile::tempdir;

    fn create_test_index() -> Index {
        let mut index = Index::new();

        // Add curl with openssl dependency
        let curl_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            sha256: "curl_hash".to_string(),
            download_url: "https://example.com/curl-8.5.0.sp".to_string(),
            minisig_url: "https://example.com/curl-8.5.0.sp.minisig".to_string(),
            dependencies: DependencyInfo {
                runtime: vec!["openssl>=3.0.0".to_string()],
                build: vec!["pkg-config>=0.29".to_string()],
            },
            sbom: None,
            description: Some("HTTP client".to_string()),
            homepage: None,
            license: None,
        };

        // Add openssl (no dependencies)
        let openssl_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            sha256: "openssl_hash".to_string(),
            download_url: "https://example.com/openssl-3.0.0.sp".to_string(),
            minisig_url: "https://example.com/openssl-3.0.0.sp.minisig".to_string(),
            dependencies: DependencyInfo::default(),
            sbom: None,
            description: Some("Crypto library".to_string()),
            homepage: None,
            license: None,
        };

        index.add_version("curl".to_string(), "8.5.0".to_string(), curl_entry);
        index.add_version("openssl".to_string(), "3.0.0".to_string(), openssl_entry);

        index
    }

    #[tokio::test]
    async fn test_basic_resolution() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let index = create_test_index();
        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        // Resolve curl (should include openssl as runtime dep)
        let context =
            ResolutionContext::new().add_runtime_dep(PackageSpec::parse("curl>=8.0.0").unwrap());

        let result = resolver.resolve(context).await.unwrap();

        // Should resolve 2 packages: curl and openssl
        assert_eq!(result.nodes.len(), 2);
        assert!(result.nodes.keys().any(|id| id.name == "curl"));
        assert!(result.nodes.keys().any(|id| id.name == "openssl"));

        // Check execution order (openssl should come before curl)
        let packages = result.packages_in_order();
        let openssl_pos = packages.iter().position(|p| p.name == "openssl").unwrap();
        let curl_pos = packages.iter().position(|p| p.name == "curl").unwrap();
        assert!(openssl_pos < curl_pos);
    }

    #[tokio::test]
    async fn test_package_not_found() {
        let temp = tempdir().unwrap();
        let mut manager = IndexManager::new(temp.path());

        let index = create_test_index();
        let json = index.to_json().unwrap();
        manager.load(Some(&json)).await.unwrap();

        let resolver = Resolver::new(manager);

        let context = ResolutionContext::new()
            .add_runtime_dep(PackageSpec::parse("nonexistent>=1.0.0").unwrap());

        let result = resolver.resolve(context).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_resolution_constraints() {
        let install_constraints = ResolutionConstraints::for_install();
        assert!(!install_constraints.include_build_deps);

        let build_constraints = ResolutionConstraints::for_build();
        assert!(build_constraints.include_build_deps);
    }
}
