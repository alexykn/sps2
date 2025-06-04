//! Advanced Starlark integration and features tests

use sps2_builder::*;
use sps2_events::Event;
use sps2_package::{execute_recipe, load_recipe};
use sps2_types::Version;
use tempfile::tempdir;
use tokio::fs;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_starlark_method_dispatch_comprehensive() {
    let temp = tempdir().unwrap();

    let recipe_content = r#"
def metadata():
    return {
        "name": "method-dispatch-test",
        "version": "1.0.0",
        "description": "Test comprehensive method dispatch"
    }

def build(ctx):
    # Test various build system methods
    fetch(ctx, "https://example.com/source.tar.gz")
    configure(ctx)
    make(ctx)
    autotools(ctx)
    cmake(ctx)
    meson(ctx)
    cargo(ctx)
    apply_patch(ctx, "fix.patch")
    command(ctx, "echo test")
    install(ctx)
"#;

    let recipe_path = temp.path().join("dispatch_test.star");
    fs::write(&recipe_path, recipe_content).await.unwrap();

    let recipe = load_recipe(&recipe_path).await.unwrap();
    let result = execute_recipe(&recipe).unwrap();

    assert_eq!(result.metadata.name, "method-dispatch-test");
    // Note: Build steps recording may be implemented in future versions
}

#[tokio::test]
async fn test_starlark_language_features() {
    let temp = tempdir().unwrap();

    let recipe_content = r#"
def metadata():
    # Test Starlark language features
    pkg_name = "lang-features-test"
    version = "2.0.0"
    
    deps = ["dep1", "dep2"] + ["dep3"]
    
    return {
        "name": pkg_name,
        "version": version,
        "description": "Test Starlark language features",
        "depends": deps
    }

def build(ctx):
    # Test variable usage and string formatting
    source_url = "https://example.com/{}-{}.tar.gz".format("source", "1.0.0")
    fetch(ctx, source_url)
    
    # Test conditionals and loops (simulated)
    if True:
        configure(ctx)
    
    for i in range(1):
        make(ctx)
    
    install(ctx)
"#;

    let recipe_path = temp.path().join("lang_features.star");
    fs::write(&recipe_path, recipe_content).await.unwrap();

    let recipe = load_recipe(&recipe_path).await.unwrap();
    let result = execute_recipe(&recipe).unwrap();

    assert_eq!(result.metadata.name, "lang-features-test");
    assert_eq!(result.metadata.version, "2.0.0");
    assert_eq!(result.metadata.runtime_deps.len(), 3);
    assert!(result.metadata.runtime_deps.contains(&"dep1".to_string()));
    assert!(result.metadata.runtime_deps.contains(&"dep2".to_string()));
    assert!(result.metadata.runtime_deps.contains(&"dep3".to_string()));
}

#[tokio::test]
async fn test_comprehensive_starlark_integration() {
    let temp = tempdir().unwrap();

    let recipe_content = r#"
def metadata():
    return {
        "name": "comprehensive-test",
        "version": "3.1.4",
        "description": "Comprehensive Starlark integration test",
        "license": "Apache-2.0",
        "homepage": "https://example.com/comprehensive",
        "depends": ["openssl>=3.0.0", "zlib>=1.2.0"],
        "build_depends": ["cmake>=3.20", "ninja>=1.10"]
    }

def build(ctx):
    # Comprehensive build workflow
    source_file = "comprehensive-test-3.1.4.tar.gz"
    fetch(ctx, "https://example.com/releases/{}".format(source_file))
    
    # Configure with options
    configure(ctx, ["--enable-ssl", "--with-zlib"])
    
    # Build with make
    make(ctx, ["-j4"])
    
    # Run tests
    command(ctx, "make test")
    
    # Install
    install(ctx)
"#;

    let recipe_path = temp.path().join("comprehensive.star");
    fs::write(&recipe_path, recipe_content).await.unwrap();

    let recipe = load_recipe(&recipe_path).await.unwrap();
    let result = execute_recipe(&recipe).unwrap();

    assert_eq!(result.metadata.name, "comprehensive-test");
    assert_eq!(result.metadata.version, "3.1.4");
    assert_eq!(result.metadata.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(
        result.metadata.homepage.as_deref(),
        Some("https://example.com/comprehensive")
    );

    assert_eq!(result.metadata.runtime_deps.len(), 2);
    assert!(result
        .metadata
        .runtime_deps
        .contains(&"openssl>=3.0.0".to_string()));
    assert!(result
        .metadata
        .runtime_deps
        .contains(&"zlib>=1.2.0".to_string()));

    assert_eq!(result.metadata.build_deps.len(), 2);
    assert!(result
        .metadata
        .build_deps
        .contains(&"cmake>=3.20".to_string()));
    assert!(result
        .metadata
        .build_deps
        .contains(&"ninja>=1.10".to_string()));
}

#[tokio::test]
async fn test_context_variable_access() {
    let temp = tempdir().unwrap();

    let recipe_content = r#"
def metadata():
    return {
        "name": "context-test",
        "version": "1.0.0",
        "description": "Test context variable access"
    }

def build(ctx):
    # Test that context methods can be called
    fetch(ctx, "https://example.com/source.tar.gz")
    configure(ctx)
    make(ctx)
    install(ctx)
"#;

    let recipe_path = temp.path().join("context_test.star");
    fs::write(&recipe_path, recipe_content).await.unwrap();

    let recipe = load_recipe(&recipe_path).await.unwrap();
    let result = execute_recipe(&recipe).unwrap();

    assert_eq!(result.metadata.name, "context-test");
}

#[tokio::test]
async fn test_builder_integration_with_actual_files() {
    let temp = tempdir().unwrap();
    let _builder = Builder::new();

    // Create a realistic build directory structure
    let build_dir = temp.path().join("build");
    fs::create_dir_all(&build_dir.join("src")).await.unwrap();
    fs::create_dir_all(&build_dir.join("include"))
        .await
        .unwrap();

    // Create test source files
    fs::write(
        build_dir.join("src").join("main.c"),
        r#"
#include <stdio.h>
int main() {
    printf("Hello from builder integration test\n");
    return 0;
}
"#,
    )
    .await
    .unwrap();

    fs::write(
        build_dir.join("include").join("test.h"),
        r#"
#ifndef TEST_H
#define TEST_H
void test_function(void);
#endif
"#,
    )
    .await
    .unwrap();

    // Create recipe that works with these files
    let recipe_content = r#"
def metadata():
    return {
        "name": "file-integration-test",
        "version": "1.0.0",
        "description": "Test builder integration with actual files"
    }

def build(ctx):
    # Test file operations
    command(ctx, "ls", "-la")
    configure(ctx)
    make(ctx)
    install(ctx)
"#;

    let recipe_path = build_dir.join("recipe.star");
    fs::write(&recipe_path, recipe_content).await.unwrap();

    let context = BuildContext::new(
        "file-integration-test".to_string(),
        Version::parse("1.0.0").unwrap(),
        recipe_path,
        build_dir,
    );

    // Verify context setup
    assert_eq!(context.name, "file-integration-test");
    assert!(context.recipe_path.exists());
    assert!(context.output_dir.exists());
}

#[tokio::test]
async fn test_event_integration() {
    let temp = tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();

    let context = BuildContext::new(
        "event-test".to_string(),
        Version::parse("1.0.0").unwrap(),
        temp.path().join("recipe.star"),
        temp.path().to_path_buf(),
    )
    .with_event_sender(tx);

    // Test that context has event sender
    assert!(context.event_sender.is_some());

    // Send a test event
    if let Some(sender) = &context.event_sender {
        sender
            .send(Event::BuildStarting {
                package: "event-test".to_string(),
                version: Version::parse("1.0.0").unwrap(),
            })
            .unwrap();
    }

    // Verify event was received
    let received_event = rx.recv().await.unwrap();
    match received_event {
        Event::BuildStarting { package, version } => {
            assert_eq!(package, "event-test");
            assert_eq!(version, Version::parse("1.0.0").unwrap());
        }
        _ => panic!("Unexpected event type"),
    }
}

#[tokio::test]
async fn test_error_handling_in_build_context() {
    let temp = tempdir().unwrap();

    // Test with non-existent recipe path
    let non_existent_recipe = temp.path().join("non_existent.star");

    // This should not panic, but the recipe won't exist for loading
    let context = BuildContext::new(
        "error-test".to_string(),
        Version::parse("1.0.0").unwrap(),
        non_existent_recipe.clone(),
        temp.path().to_path_buf(),
    );

    assert_eq!(context.name, "error-test");
    assert!(!non_existent_recipe.exists());

    // Test loading non-existent recipe should fail
    let load_result = load_recipe(&non_existent_recipe).await;
    assert!(load_result.is_err());
}

#[tokio::test]
async fn test_builder_api_comprehensive() {
    let _builder = Builder::new();

    // Test builder default configuration
    let config = BuildConfig::default();
    assert!(!config.allow_network);
    assert!(config.sbom_config.generate_spdx);

    // Test builder with custom configuration
    let custom_config = BuildConfig::with_network()
        .with_timeout(1800)
        .with_jobs(8)
        .with_sbom_config(SbomConfig::with_both_formats());

    assert!(custom_config.allow_network);
    assert_eq!(custom_config.max_build_time, Some(1800));
    assert_eq!(custom_config.build_jobs, Some(8));
}

#[tokio::test]
async fn test_builder_with_multiple_build_systems() {
    let temp = tempdir().unwrap();

    let recipe_content = r#"
def metadata():
    return {
        "name": "multi-build-test",
        "version": "1.0.0",
        "description": "Test multiple build systems"
    }

def build(ctx):
    # Test different build system support
    autotools(ctx)  # GNU Autotools
    cmake(ctx)      # CMake
    meson(ctx)      # Meson
    cargo(ctx)      # Rust Cargo
    make(ctx)       # Make
    install(ctx)
"#;

    let recipe_path = temp.path().join("multi_build.star");
    fs::write(&recipe_path, recipe_content).await.unwrap();

    let recipe = load_recipe(&recipe_path).await.unwrap();
    let result = execute_recipe(&recipe).unwrap();

    assert_eq!(result.metadata.name, "multi-build-test");
    // Build steps execution is mocked, so we just verify parsing works
}

#[tokio::test]
async fn test_starlark_advanced_features() {
    let temp = tempdir().unwrap();

    let recipe_content = r#"
def metadata():
    # Test advanced Starlark features
    base_deps = ["libc", "libm"]
    ssl_deps = ["openssl>=3.0.0"]
    all_deps = base_deps + ssl_deps
    
    build_tools = {
        "compiler": "gcc>=9.0.0",
        "build_system": "make>=4.0"
    }
    
    return {
        "name": "advanced-features",
        "version": "1.0.0",
        "description": "Advanced Starlark features test",
        "depends": all_deps,
        "build_depends": [build_tools["compiler"], build_tools["build_system"]]
    }

def build(ctx):
    # Test string operations and formatting
    pkg_name = "advanced-features"
    version = "1.0.0"
    archive_name = "{}-{}.tar.gz".format(pkg_name, version)
    url = "https://example.com/releases/{}".format(archive_name)
    
    fetch(ctx, url)
    configure(ctx, ["--prefix=/usr", "--enable-ssl"])
    make(ctx, ["-j4"])
    install(ctx)
"#;

    let recipe_path = temp.path().join("advanced.star");
    fs::write(&recipe_path, recipe_content).await.unwrap();

    let recipe = load_recipe(&recipe_path).await.unwrap();
    let result = execute_recipe(&recipe).unwrap();

    assert_eq!(result.metadata.name, "advanced-features");
    assert_eq!(result.metadata.runtime_deps.len(), 3);
    assert!(result.metadata.runtime_deps.contains(&"libc".to_string()));
    assert!(result.metadata.runtime_deps.contains(&"libm".to_string()));
    assert!(result
        .metadata
        .runtime_deps
        .contains(&"openssl>=3.0.0".to_string()));

    assert_eq!(result.metadata.build_deps.len(), 2);
    assert!(result
        .metadata
        .build_deps
        .contains(&"gcc>=9.0.0".to_string()));
    assert!(result
        .metadata
        .build_deps
        .contains(&"make>=4.0".to_string()));
}
