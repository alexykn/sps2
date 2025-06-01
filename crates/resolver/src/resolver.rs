//! Main dependency resolver implementation

use crate::graph::DependencyGraph;
use crate::{
    DepEdge, DepKind, ExecutionPlan, PackageId, ResolutionContext, ResolutionResult, ResolvedNode,
};
use semver::Version;
use sps2_errors::{Error, PackageError};
use sps2_index::{IndexManager, VersionEntry};
use sps2_manifest::Manifest;
use sps2_types::package::PackageSpec;
use sps2_types::version::VersionSpec;
use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;

/// Dependency resolver
#[derive(Clone)]
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
            let _spec = VersionSpec::from_str(dep)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use sps2_index::{DependencyInfo, Index, IndexManager, VersionEntry};
    use tempfile::tempdir;

    fn create_test_index() -> Index {
        let mut index = Index::new();

        // Add curl with openssl dependency
        let curl_entry = VersionEntry {
            revision: 1,
            arch: "arm64".to_string(),
            blake3: "curl_hash".to_string(),
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
            blake3: "openssl_hash".to_string(),
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
