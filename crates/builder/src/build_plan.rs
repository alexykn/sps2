//! Build plan representation for staged execution

use crate::environment::IsolationLevel;
use crate::recipe::model::YamlRecipe;
use crate::stages::{BuildCommand, PostStep, SourceStep};
use crate::validation;
use crate::yaml::RecipeMetadata;
use sps2_errors::Error;
use sps2_types::RpathStyle;
use std::collections::HashMap;
use std::path::Path;

/// Collection of steps by stage type
struct StageSteps {
    source: Vec<SourceStep>,
    build: Vec<BuildCommand>,
    post: Vec<PostStep>,
}

/// Complete build plan extracted from recipe
#[derive(Debug, Clone)]
pub struct BuildPlan {
    /// Package metadata
    pub metadata: RecipeMetadata,

    /// Environment configuration (extracted from recipe, applied before build)
    pub environment: EnvironmentConfig,

    /// Source operations (fetch, git, local, patches)
    pub source_steps: Vec<SourceStep>,

    /// Build operations (configure, make, etc.)
    pub build_steps: Vec<BuildCommand>,

    /// Post-processing operations
    pub post_steps: Vec<PostStep>,

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
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails for any build step
    pub fn from_yaml(
        recipe: &YamlRecipe,
        recipe_path: &Path,
        sps2_config: Option<&sps2_config::Config>,
    ) -> Result<Self, Error> {
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
        let stage_steps = Self::extract_steps_by_stage(recipe, recipe_path, sps2_config)?;

        Ok(Self {
            metadata,
            environment,
            source_steps: stage_steps.source,
            build_steps: stage_steps.build,
            post_steps: stage_steps.post,
            auto_install: recipe.install.auto,
        })
    }

    /// Extract build steps organized by stage
    fn extract_steps_by_stage(
        recipe: &YamlRecipe,
        recipe_path: &Path,
        sps2_config: Option<&sps2_config::Config>,
    ) -> Result<StageSteps, Error> {
        let source_steps = Self::extract_source_steps(recipe, recipe_path)?;
        let build_steps = Self::extract_build_steps(recipe, sps2_config)?;
        let post_steps = Self::extract_post_steps(recipe, sps2_config)?;

        Ok(StageSteps {
            source: source_steps,
            build: build_steps,
            post: post_steps,
        })
    }

    /// Extract source steps from recipe
    fn extract_source_steps(
        recipe: &YamlRecipe,
        recipe_path: &Path,
    ) -> Result<Vec<SourceStep>, Error> {
        use crate::recipe::model::{ChecksumAlgorithm, SourceMethod};

        let mut source_steps = Vec::new();

        // Source acquisition
        match &recipe.source.method {
            SourceMethod::Git { git } => {
                source_steps.push(SourceStep::Git {
                    url: git.url.clone(),
                    ref_: git.git_ref.clone(),
                });
            }
            SourceMethod::Fetch { fetch } => match &fetch.checksum {
                Some(checksum) => match &checksum.algorithm {
                    ChecksumAlgorithm::Blake3 { blake3 } => {
                        source_steps.push(SourceStep::FetchBlake3 {
                            url: fetch.url.clone(),
                            blake3: blake3.clone(),
                        });
                    }
                    ChecksumAlgorithm::Sha256 { sha256 } => {
                        source_steps.push(SourceStep::FetchSha256 {
                            url: fetch.url.clone(),
                            sha256: sha256.clone(),
                        });
                    }
                    ChecksumAlgorithm::Md5 { md5 } => {
                        source_steps.push(SourceStep::FetchMd5 {
                            url: fetch.url.clone(),
                            md5: md5.clone(),
                        });
                    }
                },
                None => {
                    source_steps.push(SourceStep::Fetch {
                        url: fetch.url.clone(),
                    });
                }
            },
            SourceMethod::Local { local } => {
                source_steps.push(SourceStep::Copy {
                    src_path: Some(local.path.clone()),
                });
            }
        }

        // Apply patches
        for patch in &recipe.source.patches {
            source_steps.push(SourceStep::ApplyPatch {
                path: patch.clone(),
            });
        }

        // Validate all source steps
        let recipe_dir = recipe_path.parent().unwrap_or(Path::new("."));
        for step in &source_steps {
            validation::validate_source_step(step, recipe_dir)?;
        }

        Ok(source_steps)
    }

    /// Extract build steps from recipe
    fn extract_build_steps(
        recipe: &YamlRecipe,
        sps2_config: Option<&sps2_config::Config>,
    ) -> Result<Vec<BuildCommand>, Error> {
        use crate::recipe::model::Build;

        let mut build_steps = Vec::new();

        match &recipe.build {
            Build::System { system, args } => {
                let step = match system {
                    crate::recipe::model::BuildSystem::Autotools => {
                        BuildCommand::Autotools { args: args.clone() }
                    }
                    crate::recipe::model::BuildSystem::Cmake => {
                        BuildCommand::Cmake { args: args.clone() }
                    }
                    crate::recipe::model::BuildSystem::Meson => {
                        BuildCommand::Meson { args: args.clone() }
                    }
                    crate::recipe::model::BuildSystem::Cargo => {
                        BuildCommand::Cargo { args: args.clone() }
                    }
                    crate::recipe::model::BuildSystem::Go => {
                        BuildCommand::Go { args: args.clone() }
                    }
                    crate::recipe::model::BuildSystem::Python => {
                        BuildCommand::Python { args: args.clone() }
                    }
                    crate::recipe::model::BuildSystem::Nodejs => {
                        BuildCommand::NodeJs { args: args.clone() }
                    }
                    crate::recipe::model::BuildSystem::Make => {
                        BuildCommand::Make { args: args.clone() }
                    }
                };
                build_steps.push(step);
            }
            Build::Steps { steps } => {
                for step in steps {
                    // Validate and convert each step
                    let build_step = validation::validate_build_step(step, sps2_config)?;
                    build_steps.push(build_step);
                }
            }
        }

        Ok(build_steps)
    }

    /// Extract post-processing steps from recipe
    fn extract_post_steps(
        recipe: &YamlRecipe,
        sps2_config: Option<&sps2_config::Config>,
    ) -> Result<Vec<PostStep>, Error> {
        use crate::recipe::model::{PostOption, RpathPatchOption};

        let mut post_steps = Vec::new();

        // Fix permissions
        match &recipe.post.fix_permissions {
            PostOption::Enabled(true) => {
                post_steps.push(PostStep::FixPermissions {
                    paths: vec![], // Will use default paths
                });
            }
            PostOption::Paths(paths) => {
                post_steps.push(PostStep::FixPermissions {
                    paths: paths.clone(),
                });
            }
            PostOption::Enabled(false) => {}
        }

        // Patch rpaths
        match &recipe.post.patch_rpaths {
            RpathPatchOption::Default => {
                // Default: Modern style (relocatable @rpath)
                post_steps.push(PostStep::PatchRpaths {
                    style: RpathStyle::Modern,
                    paths: vec![], // Will use default paths
                });
            }
            RpathPatchOption::Absolute => {
                // Absolute: Convert @rpath to absolute paths
                post_steps.push(PostStep::PatchRpaths {
                    style: RpathStyle::Absolute,
                    paths: vec![], // Will use default paths
                });
            }
            RpathPatchOption::Skip => {
                // Skip: No rpath patching
            }
        }

        // Custom post-processing commands
        for command in &recipe.post.commands {
            // Validate and convert each command
            let post_step = validation::validate_post_command(command, sps2_config)?;
            post_steps.push(post_step);
        }

        Ok(post_steps)
    }
}
