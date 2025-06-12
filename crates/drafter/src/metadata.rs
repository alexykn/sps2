//! Metadata extraction from source code

use crate::{RecipeMetadata, Result};
use serde::Deserialize;
use sps2_errors::BuildError;
use std::path::Path;
use tokio::fs;

/// Extract metadata from a source directory
pub async fn extract_metadata(source_dir: &Path) -> Result<RecipeMetadata> {
    // Try extractors in order of preference
    if let Ok(metadata) = extract_from_cargo_toml(source_dir).await {
        return Ok(metadata);
    }

    if let Ok(metadata) = extract_from_package_json(source_dir).await {
        return Ok(metadata);
    }

    if let Ok(metadata) = extract_from_pyproject_toml(source_dir).await {
        return Ok(metadata);
    }

    if let Ok(metadata) = extract_from_go_mod(source_dir).await {
        return Ok(metadata);
    }

    if let Ok(metadata) = extract_from_cmake(source_dir).await {
        return Ok(metadata);
    }

    if let Ok(metadata) = extract_from_meson(source_dir).await {
        return Ok(metadata);
    }

    if let Ok(metadata) = extract_from_autotools(source_dir).await {
        return Ok(metadata);
    }

    // Fallback: try to extract from directory name
    extract_from_directory_name(source_dir)
}

/// Cargo.toml metadata
#[derive(Deserialize)]
struct CargoToml {
    package: CargoPackage,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
    description: Option<String>,
    license: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
}

/// Extract metadata from Cargo.toml
async fn extract_from_cargo_toml(source_dir: &Path) -> Result<RecipeMetadata> {
    let cargo_toml_path = source_dir.join("Cargo.toml");
    if !cargo_toml_path.exists() {
        return Err(BuildError::DraftMetadataFailed {
            message: "Cargo.toml not found".to_string(),
        }
        .into());
    }

    let contents = fs::read_to_string(&cargo_toml_path).await.map_err(|e| {
        BuildError::DraftMetadataFailed {
            message: format!("Failed to read Cargo.toml: {e}"),
        }
    })?;

    let cargo_toml: CargoToml =
        toml::from_str(&contents).map_err(|e| BuildError::DraftMetadataFailed {
            message: format!("Failed to parse Cargo.toml: {e}"),
        })?;

    Ok(RecipeMetadata {
        name: cargo_toml.package.name,
        version: cargo_toml.package.version,
        description: cargo_toml.package.description,
        license: cargo_toml.package.license,
        homepage: cargo_toml
            .package
            .homepage
            .or(cargo_toml.package.repository),
    })
}

/// package.json metadata
#[derive(Deserialize)]
struct PackageJson {
    name: String,
    version: String,
    description: Option<String>,
    license: Option<String>,
    homepage: Option<String>,
    repository: Option<serde_json::Value>,
}

/// Extract metadata from package.json
async fn extract_from_package_json(source_dir: &Path) -> Result<RecipeMetadata> {
    let package_json_path = source_dir.join("package.json");
    if !package_json_path.exists() {
        return Err(BuildError::DraftMetadataFailed {
            message: "package.json not found".to_string(),
        }
        .into());
    }

    let contents = fs::read_to_string(&package_json_path).await.map_err(|e| {
        BuildError::DraftMetadataFailed {
            message: format!("Failed to read package.json: {e}"),
        }
    })?;

    let package_json: PackageJson =
        serde_json::from_str(&contents).map_err(|e| BuildError::DraftMetadataFailed {
            message: format!("Failed to parse package.json: {e}"),
        })?;

    // Extract homepage from repository field if needed
    let homepage = package_json.homepage.or_else(|| {
        package_json.repository.and_then(|repo| match repo {
            serde_json::Value::String(s) => Some(s),
            serde_json::Value::Object(obj) => obj
                .get("url")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            _ => None,
        })
    });

    Ok(RecipeMetadata {
        name: package_json.name,
        version: package_json.version,
        description: package_json.description,
        license: package_json.license,
        homepage,
    })
}

/// pyproject.toml metadata
#[derive(Deserialize)]
struct PyProjectToml {
    project: Option<PyProject>,
    tool: Option<PyProjectTool>,
}

#[derive(Deserialize)]
struct PyProject {
    name: String,
    version: String,
    description: Option<String>,
    license: Option<PyProjectLicense>,
    urls: Option<std::collections::HashMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PyProjectLicense {
    Simple(String),
    Complex { text: String },
}

#[derive(Deserialize)]
struct PyProjectTool {
    poetry: Option<PoetrySection>,
}

#[derive(Deserialize)]
struct PoetrySection {
    name: String,
    version: String,
    description: Option<String>,
    license: Option<String>,
    homepage: Option<String>,
    repository: Option<String>,
}

/// Extract metadata from pyproject.toml
async fn extract_from_pyproject_toml(source_dir: &Path) -> Result<RecipeMetadata> {
    let pyproject_path = source_dir.join("pyproject.toml");
    if !pyproject_path.exists() {
        return Err(BuildError::DraftMetadataFailed {
            message: "pyproject.toml not found".to_string(),
        }
        .into());
    }

    let contents =
        fs::read_to_string(&pyproject_path)
            .await
            .map_err(|e| BuildError::DraftMetadataFailed {
                message: format!("Failed to read pyproject.toml: {e}"),
            })?;

    let pyproject: PyProjectToml =
        toml::from_str(&contents).map_err(|e| BuildError::DraftMetadataFailed {
            message: format!("Failed to parse pyproject.toml: {e}"),
        })?;

    // Try [project] section first (PEP 621)
    if let Some(project) = pyproject.project {
        let license = match project.license {
            Some(PyProjectLicense::Simple(s)) => Some(s),
            Some(PyProjectLicense::Complex { text }) => Some(text),
            None => None,
        };

        let homepage = project.urls.and_then(|urls| {
            urls.get("Homepage")
                .or_else(|| urls.get("homepage"))
                .or_else(|| urls.get("Home"))
                .or_else(|| urls.get("home"))
                .cloned()
        });

        return Ok(RecipeMetadata {
            name: project.name,
            version: project.version,
            description: project.description,
            license,
            homepage,
        });
    }

    // Try [tool.poetry] section
    if let Some(tool) = pyproject.tool {
        if let Some(poetry) = tool.poetry {
            return Ok(RecipeMetadata {
                name: poetry.name,
                version: poetry.version,
                description: poetry.description,
                license: poetry.license,
                homepage: poetry.homepage.or(poetry.repository),
            });
        }
    }

    Err(BuildError::DraftMetadataFailed {
        message: "No valid metadata found in pyproject.toml".to_string(),
    }
    .into())
}

/// Extract metadata from go.mod
async fn extract_from_go_mod(source_dir: &Path) -> Result<RecipeMetadata> {
    let go_mod_path = source_dir.join("go.mod");
    if !go_mod_path.exists() {
        return Err(BuildError::DraftMetadataFailed {
            message: "go.mod not found".to_string(),
        }
        .into());
    }

    let contents =
        fs::read_to_string(&go_mod_path)
            .await
            .map_err(|e| BuildError::DraftMetadataFailed {
                message: format!("Failed to read go.mod: {e}"),
            })?;

    // Parse module name from go.mod
    let module_name = contents
        .lines()
        .find_map(|line| {
            let line = line.trim();
            if line.starts_with("module ") {
                Some(line.trim_start_matches("module ").trim())
            } else {
                None
            }
        })
        .ok_or_else(|| BuildError::DraftMetadataFailed {
            message: "Module name not found in go.mod".to_string(),
        })?;

    // Extract package name from module path
    let name = module_name
        .split('/')
        .next_back()
        .unwrap_or(module_name)
        .to_string();

    // Go doesn't have version in go.mod, we'll need to get it from elsewhere
    Ok(RecipeMetadata {
        name,
        version: "0.0.0".to_string(), // Will be overridden by git tag or user input
        description: None,
        license: None,
        homepage: Some(format!("https://{module_name}")),
    })
}

/// Extract metadata from CMakeLists.txt
async fn extract_from_cmake(source_dir: &Path) -> Result<RecipeMetadata> {
    let cmake_path = source_dir.join("CMakeLists.txt");
    if !cmake_path.exists() {
        return Err(BuildError::DraftMetadataFailed {
            message: "CMakeLists.txt not found".to_string(),
        }
        .into());
    }

    let contents =
        fs::read_to_string(&cmake_path)
            .await
            .map_err(|e| BuildError::DraftMetadataFailed {
                message: format!("Failed to read CMakeLists.txt: {e}"),
            })?;

    // Extract project name and version
    let project_regex = regex::Regex::new(r"project\s*\(\s*([^\s\)]+)(?:\s+VERSION\s+([^\s\)]+))?")
        .map_err(|e| BuildError::DraftMetadataFailed {
            message: format!("Failed to compile regex: {e}"),
        })?;

    let captures =
        project_regex
            .captures(&contents)
            .ok_or_else(|| BuildError::DraftMetadataFailed {
                message: "Project name not found in CMakeLists.txt".to_string(),
            })?;

    let name = captures
        .get(1)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| BuildError::DraftMetadataFailed {
            message: "Project name not found in CMakeLists.txt".to_string(),
        })?;

    let version = captures
        .get(2)
        .map_or_else(|| "0.0.0".to_string(), |m| m.as_str().to_string());

    Ok(RecipeMetadata {
        name,
        version,
        description: None,
        license: None,
        homepage: None,
    })
}

/// Extract metadata from meson.build
async fn extract_from_meson(source_dir: &Path) -> Result<RecipeMetadata> {
    let meson_path = source_dir.join("meson.build");
    if !meson_path.exists() {
        return Err(BuildError::DraftMetadataFailed {
            message: "meson.build not found".to_string(),
        }
        .into());
    }

    let contents =
        fs::read_to_string(&meson_path)
            .await
            .map_err(|e| BuildError::DraftMetadataFailed {
                message: format!("Failed to read meson.build: {e}"),
            })?;

    // Extract project name and version
    let project_regex =
        regex::Regex::new(r"project\s*\(\s*'([^']+)'(?:,\s*[^,]*,\s*version\s*:\s*'([^']+)')?")
            .map_err(|e| BuildError::DraftMetadataFailed {
                message: format!("Failed to compile regex: {e}"),
            })?;

    let captures =
        project_regex
            .captures(&contents)
            .ok_or_else(|| BuildError::DraftMetadataFailed {
                message: "Project name not found in meson.build".to_string(),
            })?;

    let name = captures
        .get(1)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| BuildError::DraftMetadataFailed {
            message: "Project name not found in meson.build".to_string(),
        })?;

    let version = captures
        .get(2)
        .map_or_else(|| "0.0.0".to_string(), |m| m.as_str().to_string());

    Ok(RecipeMetadata {
        name,
        version,
        description: None,
        license: None,
        homepage: None,
    })
}

/// Extract metadata from configure.ac/configure.in
async fn extract_from_autotools(source_dir: &Path) -> Result<RecipeMetadata> {
    let configure_ac = source_dir.join("configure.ac");
    let configure_in = source_dir.join("configure.in");

    let path = if configure_ac.exists() {
        configure_ac
    } else if configure_in.exists() {
        configure_in
    } else {
        return Err(BuildError::DraftMetadataFailed {
            message: "configure.ac/configure.in not found".to_string(),
        }
        .into());
    };

    let contents =
        fs::read_to_string(&path)
            .await
            .map_err(|e| BuildError::DraftMetadataFailed {
                message: format!("Failed to read {}: {}", path.display(), e),
            })?;

    // Extract AC_INIT parameters
    let init_regex = regex::Regex::new(
        r"AC_INIT\s*\(\s*\[?([^\],\)]+)\]?(?:,\s*\[?([^\],\)]+)\]?)?",
    )
    .map_err(|e| BuildError::DraftMetadataFailed {
        message: format!("Failed to compile regex: {e}"),
    })?;

    let captures =
        init_regex
            .captures(&contents)
            .ok_or_else(|| BuildError::DraftMetadataFailed {
                message: "AC_INIT not found in configure.ac".to_string(),
            })?;

    let name = captures
        .get(1)
        .map(|m| m.as_str().trim().to_string())
        .ok_or_else(|| BuildError::DraftMetadataFailed {
            message: "Package name not found in AC_INIT".to_string(),
        })?;

    let version = captures
        .get(2)
        .map_or_else(|| "0.0.0".to_string(), |m| m.as_str().trim().to_string());

    Ok(RecipeMetadata {
        name,
        version,
        description: None,
        license: None,
        homepage: None,
    })
}

/// Fallback: extract metadata from directory name
fn extract_from_directory_name(source_dir: &Path) -> Result<RecipeMetadata> {
    let dir_name = source_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| BuildError::DraftMetadataFailed {
            message: "Could not determine package name from directory".to_string(),
        })?;

    // Try to parse version from directory name (e.g., "package-1.2.3")
    let version_regex = regex::Regex::new(r"^(.+?)[-_](\d+\.\d+(?:\.\d+)?)$").map_err(|e| {
        BuildError::DraftMetadataFailed {
            message: format!("Failed to compile regex: {e}"),
        }
    })?;

    if let Some(captures) = version_regex.captures(dir_name) {
        let name = captures.get(1).unwrap().as_str().to_string();
        let version = captures.get(2).unwrap().as_str().to_string();
        Ok(RecipeMetadata {
            name,
            version,
            description: None,
            license: None,
            homepage: None,
        })
    } else {
        Ok(RecipeMetadata {
            name: dir_name.to_string(),
            version: "0.0.0".to_string(),
            description: None,
            license: None,
            homepage: None,
        })
    }
}
