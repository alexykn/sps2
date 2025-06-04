//! Example usage of the advanced dependency management system

#[cfg(test)]
mod tests {
    use crate::dependencies::*;
    use sps2_types::package::PackageSpec;

    #[tokio::test]
    async fn example_cross_compilation_dependencies() {
        // Create a dependency context for cross-compilation
        let mut context = DependencyContext::new("x86_64-apple-darwin".to_string());
        context.target_arch = Some("aarch64-linux-gnu".to_string());

        // Add build dependencies (needed on build machine)
        let gcc_cross = Dependency::new(
            PackageSpec::parse("gcc-aarch64-linux-gnu>=10.0.0").unwrap(),
            ExtendedDepKind::Build,
        );
        context.add_dependency(gcc_cross);

        // Add host dependencies (tools that run on build machine)
        let pkg_config = Dependency::new(
            PackageSpec::parse("pkg-config>=0.29.0").unwrap(),
            ExtendedDepKind::Host,
        );
        context.add_dependency(pkg_config);

        // Add target dependencies (libraries for target architecture)
        let openssl = Dependency::new(
            PackageSpec::parse("openssl>=1.1.0").unwrap(),
            ExtendedDepKind::Target,
        );
        context.add_dependency(openssl);

        // Add optional dependency with features
        let tls_backend = Dependency::new(
            PackageSpec::parse("rustls>=0.20.0").unwrap(),
            ExtendedDepKind::Target,
        )
        .with_features(vec!["tls".to_string(), "native-tls".to_string()]);
        context.add_dependency(tls_backend);

        // Enable the TLS feature
        context.enable_feature("tls");

        // In a real scenario, you would create the resolver with an actual index
        // let index_manager = IndexManager::new("/path/to/index");
        // let resolver = Resolver::new(index_manager);
        // let dep_resolver = DependencyResolver::new(resolver, None);
        // let graph = dep_resolver.resolve(&context).await.unwrap();

        // Check that we're cross-compiling
        assert!(context.is_cross_compiling());

        // Verify active dependencies
        let active_deps = context.active_dependencies();
        assert_eq!(active_deps.len(), 4); // All 4 dependencies should be active
    }

    #[test]
    fn example_feature_flags() {
        let mut context = DependencyContext::new("x86_64-apple-darwin".to_string());

        // Add always-required dependency
        let core_dep = Dependency::new(
            PackageSpec::parse("mylib-core>=1.0.0").unwrap(),
            ExtendedDepKind::Runtime,
        );
        context.add_dependency(core_dep);

        // Add optional GUI dependency
        let gui_dep = Dependency::new(
            PackageSpec::parse("mylib-gui>=1.0.0").unwrap(),
            ExtendedDepKind::Runtime,
        )
        .with_features(vec!["gui".to_string()]);
        context.add_dependency(gui_dep);

        // Add optional database dependency
        let db_dep = Dependency::new(
            PackageSpec::parse("postgresql>=13.0.0").unwrap(),
            ExtendedDepKind::Runtime,
        )
        .with_features(vec!["database".to_string(), "postgres".to_string()]);
        context.add_dependency(db_dep);

        // Without any features enabled, only core is active
        assert_eq!(context.active_dependencies().len(), 1);

        // Enable GUI feature
        context.enable_feature("gui");
        assert_eq!(context.active_dependencies().len(), 2);

        // Enable database feature
        context.enable_feature("database");
        assert_eq!(context.active_dependencies().len(), 3);
    }

    #[test]
    fn example_dependency_graph_visualization() {
        let mut graph = DependencyGraph::new();

        // Add application node
        graph.add_node(DependencyNode {
            name: "myapp".to_string(),
            version: sps2_types::Version::parse("1.0.0").unwrap(),
            dependencies: vec![],
            virtual_node: false,
        });

        // Add dependency nodes
        graph.add_node(DependencyNode {
            name: "libssl".to_string(),
            version: sps2_types::Version::parse("1.1.1").unwrap(),
            dependencies: vec![],
            virtual_node: false,
        });

        graph.add_node(DependencyNode {
            name: "zlib".to_string(),
            version: sps2_types::Version::parse("1.2.11").unwrap(),
            dependencies: vec![],
            virtual_node: false,
        });

        // Add edges
        graph.add_edge("myapp-1.0.0", "libssl-1.1.1");
        graph.add_edge("libssl-1.1.1", "zlib-1.2.11");

        // Generate DOT visualization
        let dot = graph.to_dot();
        println!("Dependency graph visualization:\n{}", dot);

        // Check topological order
        let order = graph.topological_sort().unwrap();
        assert_eq!(order.len(), 3);

        // Our topological sort returns nodes with no incoming edges first
        // Since the edges go from dependent to dependency (myapp -> libssl -> zlib),
        // myapp has no incoming edges and comes first
        let zlib_pos = order.iter().position(|x| x.contains("zlib")).unwrap();
        let ssl_pos = order.iter().position(|x| x.contains("libssl")).unwrap();
        let app_pos = order.iter().position(|x| x.contains("myapp")).unwrap();

        println!("Topological order: {:?}", order);
        println!(
            "Positions - zlib: {}, libssl: {}, myapp: {}",
            zlib_pos, ssl_pos, app_pos
        );

        // With our edge direction, the order is: myapp -> libssl -> zlib
        assert!(app_pos < ssl_pos, "myapp should come before libssl");
        assert!(ssl_pos < zlib_pos, "libssl should come before zlib");
    }
}
