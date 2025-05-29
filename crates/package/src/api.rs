//! Rhai API exposed to recipes

use crate::recipe::{BuildStep, RecipeMetadata};
use rhai::{Array, Engine, EvalAltResult};

/// Metadata builder API exposed to recipes
#[derive(Debug, Clone)]
pub struct MetadataApi {
    metadata: RecipeMetadata,
}

impl MetadataApi {
    pub fn new() -> Self {
        Self {
            metadata: RecipeMetadata::default(),
        }
    }

    pub fn name(&mut self, name: &str) -> &mut Self {
        self.metadata.name = name.to_string();
        self
    }

    pub fn version(&mut self, version: &str) -> &mut Self {
        self.metadata.version = version.to_string();
        self
    }

    pub fn description(&mut self, desc: &str) -> &mut Self {
        self.metadata.description = Some(desc.to_string());
        self
    }

    pub fn homepage(&mut self, url: &str) -> &mut Self {
        self.metadata.homepage = Some(url.to_string());
        self
    }

    pub fn license(&mut self, license: &str) -> &mut Self {
        self.metadata.license = Some(license.to_string());
        self
    }

    pub fn depends_on(&mut self, spec: &str) -> &mut Self {
        self.metadata.runtime_deps.push(spec.to_string());
        self
    }

    pub fn build_depends_on(&mut self, spec: &str) -> &mut Self {
        self.metadata.build_deps.push(spec.to_string());
        self
    }

    pub fn into_metadata(self) -> RecipeMetadata {
        self.metadata
    }
}

/// Builder API exposed to recipes
#[derive(Debug, Clone)]
pub struct BuilderApi {
    steps: Vec<BuildStep>,
    network_allowed: bool,
}

impl BuilderApi {
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            network_allowed: false,
        }
    }

    pub fn fetch(&mut self, url: &str, sha256: &str) -> &mut Self {
        self.steps.push(BuildStep::Fetch {
            url: url.to_string(),
            sha256: sha256.to_string(),
        });
        self
    }

    pub fn apply_patch(&mut self, path: &str) -> &mut Self {
        self.steps.push(BuildStep::ApplyPatch {
            path: path.to_string(),
        });
        self
    }

    pub fn allow_network(&mut self, enabled: bool) -> &mut Self {
        self.network_allowed = enabled;
        self.steps.push(BuildStep::AllowNetwork { enabled });
        self
    }

    pub fn configure(&mut self, args: Array) -> Result<&mut Self, Box<EvalAltResult>> {
        let args = array_to_strings(args)?;
        self.steps.push(BuildStep::Configure { args });
        Ok(self)
    }

    pub fn make(&mut self, args: Array) -> Result<&mut Self, Box<EvalAltResult>> {
        let args = array_to_strings(args)?;
        self.steps.push(BuildStep::Make { args });
        Ok(self)
    }

    pub fn autotools(&mut self, args: Array) -> Result<&mut Self, Box<EvalAltResult>> {
        let args = array_to_strings(args)?;
        self.steps.push(BuildStep::Autotools { args });
        Ok(self)
    }

    pub fn cmake(&mut self, args: Array) -> Result<&mut Self, Box<EvalAltResult>> {
        let args = array_to_strings(args)?;
        self.steps.push(BuildStep::Cmake { args });
        Ok(self)
    }

    pub fn meson(&mut self, args: Array) -> Result<&mut Self, Box<EvalAltResult>> {
        let args = array_to_strings(args)?;
        self.steps.push(BuildStep::Meson { args });
        Ok(self)
    }

    pub fn cargo(&mut self, args: Array) -> Result<&mut Self, Box<EvalAltResult>> {
        let args = array_to_strings(args)?;
        self.steps.push(BuildStep::Cargo { args });
        Ok(self)
    }

    pub fn command(&mut self, program: &str, args: Array) -> Result<&mut Self, Box<EvalAltResult>> {
        let args = array_to_strings(args)?;
        self.steps.push(BuildStep::Command {
            program: program.to_string(),
            args,
        });
        Ok(self)
    }

    pub fn set_env(&mut self, key: &str, value: &str) -> &mut Self {
        self.steps.push(BuildStep::SetEnv {
            key: key.to_string(),
            value: value.to_string(),
        });
        self
    }

    pub fn install(&mut self) -> &mut Self {
        self.steps.push(BuildStep::Install);
        self
    }

    pub fn into_steps(self) -> Vec<BuildStep> {
        self.steps
    }
}

/// Convert Rhai array to Vec<String>
fn array_to_strings(array: Array) -> Result<Vec<String>, Box<EvalAltResult>> {
    array
        .into_iter()
        .map(|v| {
            v.into_string()
                .map_err(|_| "array elements must be strings".into())
        })
        .collect()
}

/// Register API types with Rhai engine
pub fn register_api(engine: &mut Engine) {
    // Register MetadataApi
    engine
        .register_type_with_name::<MetadataApi>("MetadataApi")
        .register_fn("name", |api: &mut MetadataApi, name: &str| { api.name(name); })
        .register_fn("version", |api: &mut MetadataApi, version: &str| { api.version(version); })
        .register_fn("description", |api: &mut MetadataApi, desc: &str| { api.description(desc); })
        .register_fn("homepage", |api: &mut MetadataApi, url: &str| { api.homepage(url); })
        .register_fn("license", |api: &mut MetadataApi, license: &str| { api.license(license); })
        .register_fn("depends_on", |api: &mut MetadataApi, spec: &str| { api.depends_on(spec); })
        .register_fn("build_depends_on", |api: &mut MetadataApi, spec: &str| { api.build_depends_on(spec); });

    // Register BuilderApi  
    engine
        .register_type_with_name::<BuilderApi>("BuilderApi")
        .register_fn("fetch", |api: &mut BuilderApi, url: &str, sha256: &str| { api.fetch(url, sha256); })
        .register_fn("apply_patch", |api: &mut BuilderApi, path: &str| { api.apply_patch(path); })
        .register_fn("allow_network", |api: &mut BuilderApi, enabled: bool| { api.allow_network(enabled); })
        .register_fn("configure", |api: &mut BuilderApi, args: Array| -> Result<(), Box<EvalAltResult>> { api.configure(args)?; Ok(()) })
        .register_fn("make", |api: &mut BuilderApi, args: Array| -> Result<(), Box<EvalAltResult>> { api.make(args)?; Ok(()) })
        .register_fn("autotools", |api: &mut BuilderApi, args: Array| -> Result<(), Box<EvalAltResult>> { api.autotools(args)?; Ok(()) })
        .register_fn("cmake", |api: &mut BuilderApi, args: Array| -> Result<(), Box<EvalAltResult>> { api.cmake(args)?; Ok(()) })
        .register_fn("meson", |api: &mut BuilderApi, args: Array| -> Result<(), Box<EvalAltResult>> { api.meson(args)?; Ok(()) })
        .register_fn("cargo", |api: &mut BuilderApi, args: Array| -> Result<(), Box<EvalAltResult>> { api.cargo(args)?; Ok(()) })
        .register_fn("command", |api: &mut BuilderApi, program: &str, args: Array| -> Result<(), Box<EvalAltResult>> { api.command(program, args)?; Ok(()) })
        .register_fn("set_env", |api: &mut BuilderApi, key: &str, value: &str| { api.set_env(key, value); })
        .register_fn("install", |api: &mut BuilderApi| { api.install(); });
}
