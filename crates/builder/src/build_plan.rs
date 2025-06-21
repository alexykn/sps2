//! Build plan representation for staged execution

use crate::environment::IsolationLevel;
use crate::yaml::{BuildStep, PostCommand, RecipeMetadata, YamlBuildStep, YamlRecipe};
use sps2_types::RpathStyle;
use std::collections::HashMap;

/// Complete build plan extracted from recipe
#[derive(Debug, Clone)]
pub struct BuildPlan {
    /// Package metadata
    pub metadata: RecipeMetadata,

    /// Environment configuration (extracted from recipe, applied before build)
    pub environment: EnvironmentConfig,

    /// Source operations (fetch, git, local, patches)
    pub source_steps: Vec<BuildStep>,

    /// Build operations (configure, make, etc.)
    pub build_steps: Vec<BuildStep>,

    /// Post-processing operations
    pub post_steps: Vec<BuildStep>,

    /// Whether to automatically install after build
    pub auto_install: bool,
}

/// Environment configuration to apply before build
#[derive(Debug, Clone)]
pub struct EnvironmentConfig {
    /// Isolation level
    pub isolation: IsolationLevel,

    /// Whether to apply compiler defaults
    pub defaults: bool,

    /// Whether to allow network access
    pub network: bool,

    /// Environment variables to set
    pub variables: HashMap<String, String>,
}

impl BuildPlan {
    /// Create a build plan from a YAML recipe
    pub fn from_yaml(recipe: &YamlRecipe) -> Self {
        // Extract environment config
        let environment = EnvironmentConfig {
            isolation: recipe.environment.isolation,
            defaults: recipe.environment.defaults,
            network: recipe.environment.network,
            variables: recipe.environment.variables.clone(),
        };

        // Convert metadata
        let metadata = RecipeMetadata {
            name: recipe.metadata.name.clone(),
            version: recipe.metadata.version.clone(),
            description: recipe.metadata.description.clone().into(),
            homepage: recipe.metadata.homepage.clone(),
            license: Some(recipe.metadata.license.clone()),
            runtime_deps: recipe.metadata.dependencies.runtime.clone(),
            build_deps: recipe.metadata.dependencies.build.clone(),
        };

        // Extract steps by stage
        let (source_steps, build_steps, post_steps) = Self::extract_steps_by_stage(recipe);

        Self {
            metadata,
            environment,
            source_steps,
            build_steps,
            post_steps,
            auto_install: recipe.install.auto,
        }
    }

    /// Extract build steps organized by stage
    fn extract_steps_by_stage(
        recipe: &YamlRecipe,
    ) -> (Vec<BuildStep>, Vec<BuildStep>, Vec<BuildStep>) {
        let source_steps = Self::extract_source_steps(recipe);
        let build_steps = Self::extract_build_steps(recipe);
        let post_steps = Self::extract_post_steps(recipe);

        (source_steps, build_steps, post_steps)
    }

    /// Extract source steps from recipe
    fn extract_source_steps(recipe: &YamlRecipe) -> Vec<BuildStep> {
        use crate::yaml::{ChecksumAlgorithm, SourceMethod};

        let mut source_steps = Vec::new();

        // Source acquisition
        match &recipe.source.method {
            SourceMethod::Git { git } => {
                source_steps.push(BuildStep::Git {
                    url: git.url.clone(),
                    ref_: git.git_ref.clone(),
                });
            }
            SourceMethod::Fetch { fetch } => match &fetch.checksum {
                Some(checksum) => match &checksum.algorithm {
                    ChecksumAlgorithm::Blake3 { blake3 } => {
                        source_steps.push(BuildStep::FetchBlake3 {
                            url: fetch.url.clone(),
                            blake3: blake3.clone(),
                        });
                    }
                    ChecksumAlgorithm::Sha256 { sha256 } => {
                        source_steps.push(BuildStep::FetchSha256 {
                            url: fetch.url.clone(),
                            sha256: sha256.clone(),
                        });
                    }
                    ChecksumAlgorithm::Md5 { md5 } => {
                        source_steps.push(BuildStep::FetchMd5 {
                            url: fetch.url.clone(),
                            md5: md5.clone(),
                        });
                    }
                },
                None => {
                    source_steps.push(BuildStep::Fetch {
                        url: fetch.url.clone(),
                    });
                }
            },
            SourceMethod::Local { local } => {
                source_steps.push(BuildStep::Copy {
                    src_path: Some(local.path.clone()),
                });
            }
        }

        // Apply patches
        for patch in &recipe.source.patches {
            source_steps.push(BuildStep::ApplyPatch {
                path: patch.clone(),
            });
        }

        source_steps
    }

    /// Extract build steps from recipe
    fn extract_build_steps(recipe: &YamlRecipe) -> Vec<BuildStep> {
        use crate::yaml::Build;

        let mut build_steps = Vec::new();

        match &recipe.build {
            Build::System { system, args } => {
                let step = match system {
                    crate::yaml::BuildSystem::Autotools => {
                        BuildStep::Autotools { args: args.clone() }
                    }
                    crate::yaml::BuildSystem::Cmake => BuildStep::Cmake { args: args.clone() },
                    crate::yaml::BuildSystem::Meson => BuildStep::Meson { args: args.clone() },
                    crate::yaml::BuildSystem::Cargo => BuildStep::Cargo { args: args.clone() },
                    crate::yaml::BuildSystem::Go => BuildStep::Go { args: args.clone() },
                    crate::yaml::BuildSystem::Python => BuildStep::Python { args: args.clone() },
                    crate::yaml::BuildSystem::Nodejs => BuildStep::NodeJs { args: args.clone() },
                    crate::yaml::BuildSystem::Make => BuildStep::Make { args: args.clone() },
                };
                build_steps.push(step);
            }
            Build::Steps { steps } => {
                for step in steps {
                    let build_step = match step {
                        YamlBuildStep::Command { command } => {
                            let parts: Vec<&str> = command.split_whitespace().collect();
                            if !parts.is_empty() {
                                BuildStep::Command {
                                    program: parts[0].to_string(),
                                    args: parts[1..].iter().map(ToString::to_string).collect(),
                                }
                            } else {
                                continue;
                            }
                        }
                        YamlBuildStep::Shell { shell } => {
                            // Shell commands are passed directly to sh -c
                            BuildStep::Command {
                                program: "sh".to_string(),
                                args: vec!["-c".to_string(), shell.clone()],
                            }
                        }
                        YamlBuildStep::Configure { configure } => BuildStep::Configure {
                            args: configure.clone(),
                        },
                        YamlBuildStep::Make { make } => BuildStep::Make { args: make.clone() },
                        YamlBuildStep::Cmake { cmake } => BuildStep::Cmake {
                            args: cmake.clone(),
                        },
                        YamlBuildStep::Meson { meson } => BuildStep::Meson {
                            args: meson.clone(),
                        },
                        YamlBuildStep::Cargo { cargo } => BuildStep::Cargo {
                            args: cargo.clone(),
                        },
                        YamlBuildStep::Go { go } => BuildStep::Go { args: go.clone() },
                        YamlBuildStep::Python { python } => BuildStep::Python {
                            args: python.clone(),
                        },
                        YamlBuildStep::Nodejs { nodejs } => BuildStep::NodeJs {
                            args: nodejs.clone(),
                        },
                    };
                    build_steps.push(build_step);
                }
            }
        }

        build_steps
    }

    /// Extract post-processing steps from recipe
    fn extract_post_steps(recipe: &YamlRecipe) -> Vec<BuildStep> {
        use crate::yaml::PostOption;

        let mut post_steps = Vec::new();

        // Fix permissions
        match &recipe.post.fix_permissions {
            PostOption::Enabled(true) => {
                post_steps.push(BuildStep::FixPermissions {
                    paths: vec![], // Will use default paths
                });
            }
            PostOption::Paths(paths) => {
                post_steps.push(BuildStep::FixPermissions {
                    paths: paths.clone(),
                });
            }
            PostOption::Enabled(false) => {}
        }

        // Patch rpaths
        match &recipe.post.patch_rpaths {
            PostOption::Enabled(true) => {
                post_steps.push(BuildStep::PatchRpaths {
                    style: RpathStyle::Modern, // Default style
                    paths: vec![],             // Will use default paths
                });
            }
            PostOption::Paths(paths) => {
                post_steps.push(BuildStep::PatchRpaths {
                    style: RpathStyle::Modern, // Default style
                    paths: paths.clone(),
                });
            }
            PostOption::Enabled(false) => {}
        }

        // Custom post-processing commands
        for command in &recipe.post.commands {
            match command {
                PostCommand::Simple(cmd) => {
                    // Simple commands: split by whitespace
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    if !parts.is_empty() {
                        post_steps.push(BuildStep::Command {
                            program: parts[0].to_string(),
                            args: parts[1..].iter().map(ToString::to_string).collect(),
                        });
                    }
                }
                PostCommand::Shell { shell } => {
                    // Shell commands: passed to sh -c
                    post_steps.push(BuildStep::Command {
                        program: "sh".to_string(),
                        args: vec!["-c".to_string(), shell.clone()],
                    });
                }
            }
        }

        post_steps
    }
}
