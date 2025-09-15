//! YAML recipe generation

use crate::{BuildInfo, RecipeMetadata, Result, SourceLocation};
use sps2_errors::BuildError;
use sps2_types::{
    Build, BuildSystem, Dependencies, Environment, FetchSource, GitSource, Install, IsolationLevel,
    LocalSource, Metadata, ParsedStep, Post, Source, SourceMethod, YamlRecipe,
};
use std::collections::HashMap;

/// Generate YAML recipe from drafter metadata and build info
pub fn generate_yaml_recipe(
    metadata: &RecipeMetadata,
    build_info: &BuildInfo,
    source: &SourceLocation,
) -> Result<String> {
    // Create YAML recipe structure
    let yaml_recipe = YamlRecipe {
        metadata: convert_metadata(metadata, build_info),
        facts: convert_facts(metadata, build_info), // Add dynamic facts
        environment: convert_environment(build_info),
        source: convert_source(source),
        build: convert_build(build_info),
        post: convert_post(build_info),
        install: convert_install(), // Smart install defaults
    };

    // Serialize to YAML
    serde_yaml2::to_string(&yaml_recipe).map_err(|e| {
        BuildError::DraftTemplateFailed {
            message: format!("Failed to serialize YAML recipe: {e}"),
        }
        .into()
    })
}

/// Convert drafter metadata to YAML metadata
fn convert_metadata(metadata: &RecipeMetadata, build_info: &BuildInfo) -> Metadata {
    // Convert detected dependencies to runtime/build deps
    let mut runtime_deps = Vec::new();
    let mut build_deps = Vec::new();

    for dep in &build_info.dependencies {
        if dep.build_time {
            build_deps.push(dep.sps2_name.clone());
        } else {
            runtime_deps.push(dep.sps2_name.clone());
        }
    }

    Metadata {
        name: metadata.name.clone(),
        version: metadata.version.clone(),
        description: metadata
            .description
            .clone()
            .unwrap_or_else(|| "TODO: Add package description".to_string()),
        license: metadata
            .license
            .clone()
            .unwrap_or_else(|| "TODO: Specify license".to_string()),
        homepage: metadata.homepage.clone(),
        dependencies: if runtime_deps.is_empty() && build_deps.is_empty() {
            Dependencies::default()
        } else {
            Dependencies {
                runtime: runtime_deps,
                build: build_deps,
            }
        },
    }
}

/// Convert build info to environment configuration
fn convert_environment(build_info: &BuildInfo) -> Environment {
    Environment {
        isolation: IsolationLevel::default(), // Use default isolation level
        defaults: should_use_defaults(build_info),
        network: build_info.needs_network,
        variables: HashMap::new(), // TODO: Add environment variables
    }
}

/// Determine if compiler defaults should be used
fn should_use_defaults(build_info: &BuildInfo) -> bool {
    // Use defaults for C/C++ projects that can benefit from optimization
    matches!(
        build_info.build_system.as_str(),
        "autotools" | "cmake" | "meson" | "make"
    )
}

/// Convert source location to YAML source specification
fn convert_source(source: &SourceLocation) -> Source {
    let method = match source {
        SourceLocation::Git(url) => SourceMethod::Git {
            git: GitSource {
                url: url.clone(),
                git_ref: "HEAD".to_string(), // TODO: Support specific refs
            },
        },
        SourceLocation::Url(url) => SourceMethod::Fetch {
            fetch: FetchSource {
                url: url.clone(),
                checksum: None, // TODO: Add checksum support
                extract_to: None,
            },
        },
        SourceLocation::Local(path) | SourceLocation::Archive(path) => SourceMethod::Local {
            local: LocalSource {
                path: path.display().to_string(),
            },
        },
    };

    Source {
        method: Some(method),
        sources: Vec::new(), // Empty for single source (backward compatibility)
        patches: Vec::new(), // TODO: Add patch support
    }
}

/// Convert build info to YAML build specification
fn convert_build(build_info: &BuildInfo) -> Build {
    // Map drafter build system names to YAML build system enum
    let system = match build_info.build_system.as_str() {
        "autotools" => BuildSystem::Autotools,
        "cmake" => BuildSystem::Cmake,
        "meson" => BuildSystem::Meson,
        "cargo" => BuildSystem::Cargo,
        "go" => BuildSystem::Go,
        "python" => BuildSystem::Python,
        "nodejs" => BuildSystem::Nodejs,
        "make" => BuildSystem::Make,
        _ => {
            // For unknown build systems, use custom steps
            return Build::Steps {
                steps: vec![ParsedStep::Shell {
                    shell: format!(
                        "# TODO: Build system '{}' could not be mapped automatically",
                        build_info.build_system
                    ),
                }],
            };
        }
    };

    Build::System {
        system,
        args: build_info.build_args.clone(),
    }
}

/// Convert dynamic facts/variables
fn convert_facts(metadata: &RecipeMetadata, build_info: &BuildInfo) -> HashMap<String, String> {
    let mut facts = HashMap::new();

    // Add common build variables that might be useful in complex builds
    if matches!(
        build_info.build_system.as_str(),
        "autotools" | "cmake" | "meson"
    ) {
        facts.insert(
            "CONFIGURE_ARGS".to_string(),
            build_info.build_args.join(" "),
        );
    }

    // Add package-specific facts that templates often use
    facts.insert("PACKAGE_NAME".to_string(), metadata.name.clone());
    facts.insert("PACKAGE_VERSION".to_string(), metadata.version.clone());

    facts
}

/// Convert install behavior
fn convert_install() -> Install {
    // Use default (auto: false) to make YAML cleaner
    Install::default()
}

/// Convert build info to post-processing configuration
fn convert_post(build_info: &BuildInfo) -> Post {
    // Determine if we need special post-processing based on build system
    let needs_permission_fix = matches!(build_info.build_system.as_str(), "go" | "cargo");
    let rpath_style = match build_info.build_system.as_str() {
        "autotools" | "cmake" | "meson" => Some("default".to_string()),
        _ => None, // Language-specific build systems usually don't need rpath patching
    };

    Post {
        patch_rpaths: rpath_style,
        fix_permissions: if needs_permission_fix {
            Some(sps2_types::PostOption::Boolean(true))
        } else {
            None
        },
        qa_pipeline: sps2_types::QaPipelineOverride::Auto, // Use auto-detection
        commands: Vec::new(),                              // No custom post commands for now
    }
}
