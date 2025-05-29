//! Sandboxed Rhai execution environment

use crate::api::{register_api, BuilderApi, MetadataApi};
use crate::recipe::{BuildStep, Recipe, RecipeMetadata};
use rhai::{Engine, OptimizationLevel, Scope};
use spsv2_errors::{BuildError, Error};

/// Result of recipe execution
#[derive(Debug)]
pub struct RecipeResult {
    pub metadata: RecipeMetadata,
    pub build_steps: Vec<BuildStep>,
}

/// Sandboxed recipe execution engine
pub struct RecipeEngine {
    engine: Engine,
}

impl RecipeEngine {
    /// Create a new sandboxed engine
    pub fn new() -> Self {
        let mut engine = Engine::new();

        // Configure sandbox restrictions
        engine.set_max_operations(50_000_000); // 50M operations max
        engine.set_max_expr_depths(100, 100); // Max expression depth
        engine.set_max_string_size(10_000_000); // 10MB max string
        engine.set_max_array_size(10_000); // Max array elements
        engine.set_max_map_size(10_000); // Max map entries
        engine.set_optimization_level(OptimizationLevel::Simple);

        // Disable dangerous features
        engine.set_allow_if_expression(true);
        engine.set_allow_switch_expression(true);
        engine.set_allow_loop_expressions(true);
        engine.set_allow_statement_expression(true);

        // Register our API
        register_api(&mut engine);

        Self { engine }
    }

    /// Execute a recipe
    pub fn execute(&self, recipe: &Recipe) -> Result<RecipeResult, Error> {
        let mut scope = Scope::new();

        // Create the wrapper script that captures API calls
        let wrapper_script = format!(
            r#"
            // Global variables to store results
            let metadata_result = #{{}};
            let build_steps = [];
            
            // Wrapper for metadata API
            fn metadata(m) {{
                m.name = |name| {{ metadata_result.name = name; m }};
                m.version = |version| {{ metadata_result.version = version; m }};
                m.description = |desc| {{ metadata_result.description = desc; m }};
                m.homepage = |url| {{ metadata_result.homepage = url; m }};
                m.license = |lic| {{ metadata_result.license = lic; m }};
                m.depends_on = |spec| {{ 
                    if !metadata_result.contains("runtime_deps") {{
                        metadata_result.runtime_deps = [];
                    }}
                    metadata_result.runtime_deps.push(spec);
                    m 
                }};
                m.build_depends_on = |spec| {{ 
                    if !metadata_result.contains("build_deps") {{
                        metadata_result.build_deps = [];
                    }}
                    metadata_result.build_deps.push(spec);
                    m 
                }};
                
                // Call the function with the API object
                m
            }}
            
            // Wrapper for builder API
            fn build(b) {{
                b.fetch = |url, sha256| {{ 
                    build_steps.push(#{{"type": "fetch", "url": url, "sha256": sha256}});
                    b 
                }};
                b.apply_patch = |path| {{ 
                    build_steps.push(#{{"type": "apply_patch", "path": path}});
                    b 
                }};
                b.allow_network = |enabled| {{ 
                    build_steps.push(#{{"type": "allow_network", "enabled": enabled}});
                    b 
                }};
                b.configure = |args| {{ 
                    build_steps.push(#{{"type": "configure", "args": args}});
                    b 
                }};
                b.make = |args| {{ 
                    build_steps.push(#{{"type": "make", "args": args}});
                    b 
                }};
                b.autotools = |args| {{ 
                    build_steps.push(#{{"type": "autotools", "args": args}});
                    b 
                }};
                b.cmake = |args| {{ 
                    build_steps.push(#{{"type": "cmake", "args": args}});
                    b 
                }};
                b.meson = |args| {{ 
                    build_steps.push(#{{"type": "meson", "args": args}});
                    b 
                }};
                b.cargo = |args| {{ 
                    build_steps.push(#{{"type": "cargo", "args": args}});
                    b 
                }};
                b.command = |program, args| {{ 
                    build_steps.push(#{{"type": "command", "program": program, "args": args}});
                    b 
                }};
                b.set_env = |key, value| {{ 
                    build_steps.push(#{{"type": "set_env", "key": key, "value": value}});
                    b 
                }};
                b.install = || {{ 
                    build_steps.push(#{{"type": "install"}});
                    b 
                }};
                
                // Call the function with the API object
                b
            }}
            
            // User recipe starts here
            {}
            
            // Return results
            #{{"metadata": metadata_result, "build_steps": build_steps}}
        "#,
            recipe.content
        );

        // Execute the wrapped script
        let result: rhai::Map = self
            .engine
            .eval_with_scope(&mut scope, &wrapper_script)
            .map_err(|e| BuildError::RecipeError {
                message: format!("recipe execution failed: {e}"),
            })?;

        // Extract and convert metadata
        let metadata_map = result
            .get("metadata")
            .and_then(|v| v.clone().try_cast::<rhai::Map>())
            .ok_or_else(|| BuildError::RecipeError {
                message: "failed to extract metadata".to_string(),
            })?;

        let metadata = self.extract_metadata(metadata_map)?;

        // Extract and convert build steps
        let steps_array = result
            .get("build_steps")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
            .ok_or_else(|| BuildError::RecipeError {
                message: "failed to extract build steps".to_string(),
            })?;

        let build_steps = self.extract_build_steps(steps_array)?;

        // Validate results
        if metadata.name.is_empty() {
            return Err(BuildError::RecipeError {
                message: "package name not set".to_string(),
            }
            .into());
        }

        if metadata.version.is_empty() {
            return Err(BuildError::RecipeError {
                message: "package version not set".to_string(),
            }
            .into());
        }

        if !build_steps.iter().any(|s| matches!(s, BuildStep::Install)) {
            return Err(BuildError::RecipeError {
                message: "build() must call install()".to_string(),
            }
            .into());
        }

        Ok(RecipeResult {
            metadata,
            build_steps,
        })
    }

    fn extract_metadata(&self, map: rhai::Map) -> Result<RecipeMetadata, Error> {
        let mut metadata = RecipeMetadata::default();

        if let Some(name) = map.get("name").and_then(|v| v.clone().into_string().ok()) {
            metadata.name = name;
        }

        if let Some(version) = map
            .get("version")
            .and_then(|v| v.clone().into_string().ok())
        {
            metadata.version = version;
        }

        if let Some(desc) = map
            .get("description")
            .and_then(|v| v.clone().into_string().ok())
        {
            metadata.description = Some(desc);
        }

        if let Some(homepage) = map
            .get("homepage")
            .and_then(|v| v.clone().into_string().ok())
        {
            metadata.homepage = Some(homepage);
        }

        if let Some(license) = map
            .get("license")
            .and_then(|v| v.clone().into_string().ok())
        {
            metadata.license = Some(license);
        }

        if let Some(deps) = map
            .get("runtime_deps")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
        {
            for dep in deps {
                if let Ok(spec) = dep.into_string() {
                    metadata.runtime_deps.push(spec);
                }
            }
        }

        if let Some(deps) = map
            .get("build_deps")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
        {
            for dep in deps {
                if let Ok(spec) = dep.into_string() {
                    metadata.build_deps.push(spec);
                }
            }
        }

        Ok(metadata)
    }

    fn extract_build_steps(&self, array: rhai::Array) -> Result<Vec<BuildStep>, Error> {
        let mut steps = Vec::new();

        for item in array {
            if let Some(map) = item.try_cast::<rhai::Map>() {
                let step_type = map
                    .get("type")
                    .and_then(|v| v.clone().into_string().ok())
                    .ok_or_else(|| BuildError::RecipeError {
                        message: "build step missing type".to_string(),
                    })?;

                let step = match step_type.as_str() {
                    "fetch" => {
                        let url = map
                            .get("url")
                            .and_then(|v| v.clone().into_string().ok())
                            .ok_or_else(|| BuildError::RecipeError {
                                message: "fetch missing url".to_string(),
                            })?;
                        let sha256 = map
                            .get("sha256")
                            .and_then(|v| v.clone().into_string().ok())
                            .ok_or_else(|| BuildError::RecipeError {
                                message: "fetch missing sha256".to_string(),
                            })?;
                        BuildStep::Fetch { url, sha256 }
                    }
                    "apply_patch" => {
                        let path = map
                            .get("path")
                            .and_then(|v| v.clone().into_string().ok())
                            .ok_or_else(|| BuildError::RecipeError {
                                message: "apply_patch missing path".to_string(),
                            })?;
                        BuildStep::ApplyPatch { path }
                    }
                    "allow_network" => {
                        let enabled = map
                            .get("enabled")
                            .and_then(|v| v.clone().as_bool().ok())
                            .unwrap_or(false);
                        BuildStep::AllowNetwork { enabled }
                    }
                    "configure" => {
                        let args = self.extract_string_array(&map, "args")?;
                        BuildStep::Configure { args }
                    }
                    "make" => {
                        let args = self.extract_string_array(&map, "args")?;
                        BuildStep::Make { args }
                    }
                    "autotools" => {
                        let args = self.extract_string_array(&map, "args")?;
                        BuildStep::Autotools { args }
                    }
                    "cmake" => {
                        let args = self.extract_string_array(&map, "args")?;
                        BuildStep::Cmake { args }
                    }
                    "meson" => {
                        let args = self.extract_string_array(&map, "args")?;
                        BuildStep::Meson { args }
                    }
                    "cargo" => {
                        let args = self.extract_string_array(&map, "args")?;
                        BuildStep::Cargo { args }
                    }
                    "command" => {
                        let program = map
                            .get("program")
                            .and_then(|v| v.clone().into_string().ok())
                            .ok_or_else(|| BuildError::RecipeError {
                                message: "command missing program".to_string(),
                            })?;
                        let args = self.extract_string_array(&map, "args")?;
                        BuildStep::Command { program, args }
                    }
                    "set_env" => {
                        let key = map
                            .get("key")
                            .and_then(|v| v.clone().into_string().ok())
                            .ok_or_else(|| BuildError::RecipeError {
                                message: "set_env missing key".to_string(),
                            })?;
                        let value = map
                            .get("value")
                            .and_then(|v| v.clone().into_string().ok())
                            .ok_or_else(|| BuildError::RecipeError {
                                message: "set_env missing value".to_string(),
                            })?;
                        BuildStep::SetEnv { key, value }
                    }
                    "install" => BuildStep::Install,
                    _ => {
                        return Err(BuildError::RecipeError {
                            message: format!("unknown build step type: {step_type}"),
                        }
                        .into());
                    }
                };

                steps.push(step);
            }
        }

        Ok(steps)
    }

    fn extract_string_array(&self, map: &rhai::Map, key: &str) -> Result<Vec<String>, Error> {
        let array = map
            .get(key)
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
            .ok_or_else(|| BuildError::RecipeError {
                message: format!("{key} must be an array"),
            })?;

        let mut strings = Vec::new();
        for item in array {
            if let Ok(s) = item.into_string() {
                strings.push(s);
            } else {
                return Err(BuildError::RecipeError {
                    message: format!("{key} array must contain only strings"),
                }
                .into());
            }
        }

        Ok(strings)
    }
}

impl Default for RecipeEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_recipe() {
        let recipe_content = r#"
            fn metadata(m) {
                m.name("test-pkg")
                 .version("1.0.0")
                 .description("Test package");
            }
            
            fn build(b) {
                b.fetch("https://example.com/src.tar.gz", "abc123")
                 .configure(["--prefix=$PREFIX"])
                 .make(["-j$JOBS"])
                 .install();
            }
        "#;

        let recipe = Recipe::parse(recipe_content).unwrap();
        let engine = RecipeEngine::new();
        let result = engine.execute(&recipe).unwrap();

        assert_eq!(result.metadata.name, "test-pkg");
        assert_eq!(result.metadata.version, "1.0.0");
        assert_eq!(result.build_steps.len(), 4);
    }
}
