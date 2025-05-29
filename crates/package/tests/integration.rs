//! Integration tests for package crate

#[cfg(test)]
mod tests {
    use spsv2_package::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_load_recipe_file() {
        let temp = tempdir().unwrap();
        let recipe_path = temp.path().join("test.rhai");

        let recipe_content = r#"
fn metadata(m) {
    m.name("curl")
     .version("8.5.0")
     .description("Command line HTTP client")
     .homepage("https://curl.se")
     .license("MIT");
     
    m.depends_on("openssl>=3.0.0")
     .depends_on("zlib~=1.2.0");
     
    m.build_depends_on("pkg-config>=0.29")
     .build_depends_on("perl>=5.0");
}

fn build(b) {
    b.fetch(
        "https://curl.se/download/curl-8.5.0.tar.gz",
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
     )
     .configure([
        "--prefix=$PREFIX",
        "--with-openssl",
        "--enable-threaded-resolver"
     ])
     .make(["-j$JOBS"])
     .install();
}
"#;

        tokio::fs::write(&recipe_path, recipe_content)
            .await
            .unwrap();

        // Load recipe
        let recipe = load_recipe(&recipe_path).await.unwrap();

        // Execute recipe
        let result = execute_recipe(&recipe).unwrap();

        // Verify metadata
        assert_eq!(result.metadata.name, "curl");
        assert_eq!(result.metadata.version, "8.5.0");
        assert_eq!(
            result.metadata.description.as_deref(),
            Some("Command line HTTP client")
        );
        assert_eq!(result.metadata.homepage.as_deref(), Some("https://curl.se"));
        assert_eq!(result.metadata.license.as_deref(), Some("MIT"));
        assert_eq!(result.metadata.runtime_deps.len(), 2);
        assert_eq!(result.metadata.build_deps.len(), 2);

        // Verify build steps
        assert_eq!(result.build_steps.len(), 4);
        assert!(matches!(&result.build_steps[0], BuildStep::Fetch { .. }));
        assert!(matches!(
            &result.build_steps[1],
            BuildStep::Configure { .. }
        ));
        assert!(matches!(&result.build_steps[2], BuildStep::Make { .. }));
        assert!(matches!(&result.build_steps[3], BuildStep::Install));
    }

    #[test]
    fn test_recipe_with_network() {
        let recipe_content = r#"
fn metadata(m) {
    m.name("nodejs")
     .version("20.11.0");
}

fn build(b) {
    b.allow_network(true)
     .fetch(
        "https://nodejs.org/dist/v20.11.0/node-v20.11.0.tar.gz",
        "1234567890"
     )
     .configure(["--prefix=$PREFIX"])
     .make([])
     .install();
}
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let result = execute_recipe(&recipe).unwrap();

        // Check for network allow step
        assert!(result
            .build_steps
            .iter()
            .any(|s| matches!(s, BuildStep::AllowNetwork { enabled: true })));
    }

    #[test]
    fn test_recipe_with_patches() {
        let recipe_content = r#"
fn metadata(m) {
    m.name("patched-pkg")
     .version("1.0.0");
}

fn build(b) {
    b.fetch("https://example.com/src.tar.gz", "abc123")
     .apply_patch("fix-build.patch")
     .apply_patch("security.patch")
     .autotools(["--prefix=$PREFIX"])
     .install();
}
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let result = execute_recipe(&recipe).unwrap();

        // Count patch steps
        let patch_count = result
            .build_steps
            .iter()
            .filter(|s| matches!(s, BuildStep::ApplyPatch { .. }))
            .count();
        assert_eq!(patch_count, 2);
    }

    #[test]
    fn test_recipe_with_cmake() {
        let recipe_content = r#"
fn metadata(m) {
    m.name("cmake-pkg")
     .version("2.0.0");
}

fn build(b) {
    b.fetch("https://example.com/src.tar.gz", "def456")
     .cmake([
        "-DCMAKE_BUILD_TYPE=Release",
        "-DCMAKE_INSTALL_PREFIX=$PREFIX",
        "-DENABLE_TESTS=OFF"
     ])
     .install();
}
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let result = execute_recipe(&recipe).unwrap();

        // Verify cmake step
        assert!(result.build_steps.iter().any(|s| {
            if let BuildStep::Cmake { args } = s {
                args.contains(&"-DCMAKE_BUILD_TYPE=Release".to_string())
            } else {
                false
            }
        }));
    }

    #[test]
    fn test_recipe_validation_errors() {
        // Missing metadata function
        let recipe_content = r#"
fn build(b) {
    b.install();
}
"#;
        assert!(Recipe::parse(recipe_content).is_err());

        // Missing build function
        let recipe_content = r#"
fn metadata(m) {
    m.name("test").version("1.0");
}
"#;
        assert!(Recipe::parse(recipe_content).is_err());

        // Missing name
        let recipe_content = r#"
fn metadata(m) {
    m.version("1.0");
}
fn build(b) {
    b.install();
}
"#;
        let recipe = Recipe::parse(recipe_content).unwrap();
        assert!(execute_recipe(&recipe).is_err());

        // Missing install
        let recipe_content = r#"
fn metadata(m) {
    m.name("test").version("1.0");
}
fn build(b) {
    b.fetch("https://example.com/src.tar.gz", "abc");
}
"#;
        let recipe = Recipe::parse(recipe_content).unwrap();
        assert!(execute_recipe(&recipe).is_err());
    }

    #[test]
    fn test_recipe_with_env_vars() {
        let recipe_content = r#"
fn metadata(m) {
    m.name("env-test")
     .version("1.0.0");
}

fn build(b) {
    b.set_env("CC", "clang")
     .set_env("CFLAGS", "-O3 -march=native")
     .fetch("https://example.com/src.tar.gz", "xyz789")
     .configure(["--prefix=$PREFIX"])
     .make([])
     .install();
}
"#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let result = execute_recipe(&recipe).unwrap();

        // Count env steps
        let env_count = result
            .build_steps
            .iter()
            .filter(|s| matches!(s, BuildStep::SetEnv { .. }))
            .count();
        assert_eq!(env_count, 2);
    }
}
