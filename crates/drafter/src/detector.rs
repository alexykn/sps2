//! Build system detection

use crate::{BuildInfo, Result};
use sps2_errors::BuildError;
use std::path::Path;
use walkdir::WalkDir;

/// Build system detection result with confidence score
#[derive(Debug, Clone)]
struct DetectionResult {
    build_system: String,
    build_function: String,
    build_args: Vec<String>,
    needs_network: bool,
    confidence: u8, // 0-100
}

/// Detect build system and extract build information
pub async fn detect(source_dir: &Path) -> Result<BuildInfo> {
    let detectors = vec![
        detect_cargo(source_dir).await,
        detect_cmake(source_dir).await,
        detect_autotools(source_dir),
        detect_meson(source_dir).await,
        detect_python(source_dir).await,
        detect_go(source_dir),
        detect_nodejs(source_dir).await,
    ];

    // Filter out errors and sort by confidence
    let mut valid_detections: Vec<DetectionResult> = detectors
        .into_iter()
        .filter_map(std::result::Result::ok)
        .collect();

    valid_detections.sort_by(|a, b| b.confidence.cmp(&a.confidence));

    // Return the highest confidence detection
    if let Some(best) = valid_detections.first() {
        Ok(BuildInfo {
            build_system: best.build_system.clone(),
            build_function: best.build_function.clone(),
            build_args: best.build_args.clone(),
            dependencies: Vec::new(), // Will be filled in later
            needs_network: best.needs_network,
        })
    } else {
        // No build system detected
        Ok(BuildInfo {
            build_system: "unknown".to_string(),
            build_function: "# TODO: Add build commands".to_string(),
            build_args: Vec::new(),
            dependencies: Vec::new(),
            needs_network: false,
        })
    }
}

/// Detect Cargo (Rust) build system
async fn detect_cargo(source_dir: &Path) -> Result<DetectionResult> {
    let cargo_toml = source_dir.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Err(BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
        .into());
    }

    // Check for workspace vs single package
    let contents = tokio::fs::read_to_string(&cargo_toml).await.map_err(|_| {
        BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
    })?;

    let is_workspace = contents.contains("[workspace]");
    let confidence = if is_workspace { 90 } else { 95 };

    Ok(DetectionResult {
        build_system: "cargo".to_string(),
        build_function: "cargo".to_string(),
        build_args: vec!["--release".to_string()],
        needs_network: true, // Cargo needs network for dependencies
        confidence,
    })
}

/// Detect `CMake` build system
async fn detect_cmake(source_dir: &Path) -> Result<DetectionResult> {
    let cmake_lists = source_dir.join("CMakeLists.txt");
    if !cmake_lists.exists() {
        return Err(BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
        .into());
    }

    // Check for common CMake patterns to increase confidence
    let contents = tokio::fs::read_to_string(&cmake_lists).await.map_err(|_| {
        BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
    })?;

    let mut confidence = 80;
    if contents.contains("project(") {
        confidence += 10;
    }
    if contents.contains("add_executable(") || contents.contains("add_library(") {
        confidence += 5;
    }

    Ok(DetectionResult {
        build_system: "cmake".to_string(),
        build_function: "cmake".to_string(),
        build_args: vec!["-DCMAKE_BUILD_TYPE=Release".to_string()],
        needs_network: false, // CMake itself doesn't need network usually
        confidence,
    })
}

/// Detect Autotools build system
fn detect_autotools(source_dir: &Path) -> Result<DetectionResult> {
    let configure_ac = source_dir.join("configure.ac");
    let configure_in = source_dir.join("configure.in");
    let configure = source_dir.join("configure");
    let makefile_am = source_dir.join("Makefile.am");
    let makefile_in = source_dir.join("Makefile.in");

    let mut confidence = 0;

    if configure_ac.exists() || configure_in.exists() {
        confidence += 40;
    }
    if configure.exists() && configure.metadata().is_ok_and(|m| m.is_file()) {
        confidence += 30;
    }
    if makefile_am.exists() {
        confidence += 20;
    }
    if makefile_in.exists() {
        confidence += 10;
    }

    if confidence == 0 {
        return Err(BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
        .into());
    }

    Ok(DetectionResult {
        build_system: "autotools".to_string(),
        build_function: "autotools".to_string(),
        build_args: Vec::new(),
        needs_network: false,
        confidence,
    })
}

/// Detect Meson build system
async fn detect_meson(source_dir: &Path) -> Result<DetectionResult> {
    let meson_build = source_dir.join("meson.build");
    if !meson_build.exists() {
        return Err(BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
        .into());
    }

    // Check for project() call to increase confidence
    let contents = tokio::fs::read_to_string(&meson_build).await.map_err(|_| {
        BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
    })?;

    let mut confidence = 85;
    if contents.contains("project(") {
        confidence += 10;
    }

    Ok(DetectionResult {
        build_system: "meson".to_string(),
        build_function: "meson".to_string(),
        build_args: vec!["--buildtype=release".to_string()],
        needs_network: false,
        confidence,
    })
}

/// Detect Python build system
async fn detect_python(source_dir: &Path) -> Result<DetectionResult> {
    let pyproject_toml = source_dir.join("pyproject.toml");
    let setup_py = source_dir.join("setup.py");
    let setup_cfg = source_dir.join("setup.cfg");

    let mut confidence = 0;
    let mut build_system = "python";

    if pyproject_toml.exists() {
        confidence += 50;
        // Check if it uses a specific build backend
        if let Ok(contents) = tokio::fs::read_to_string(&pyproject_toml).await {
            if contents.contains("poetry") {
                build_system = "poetry";
                confidence += 20;
            } else if contents.contains("hatchling") {
                build_system = "hatch";
                confidence += 15;
            } else if contents.contains("setuptools") {
                confidence += 10;
            }
        }
    }

    if setup_py.exists() {
        confidence += 30;
    }

    if setup_cfg.exists() {
        confidence += 10;
    }

    // Look for Python files to confirm this is a Python project
    let has_python_files = WalkDir::new(source_dir)
        .max_depth(2)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .any(|entry| {
            entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "py")
        });

    if has_python_files {
        confidence += 15;
    }

    if confidence < 30 {
        return Err(BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
        .into());
    }

    Ok(DetectionResult {
        build_system: build_system.to_string(),
        build_function: "python".to_string(),
        build_args: Vec::new(),
        needs_network: true, // Python builds often need to download dependencies
        confidence,
    })
}

/// Detect Go build system
fn detect_go(source_dir: &Path) -> Result<DetectionResult> {
    let go_mod = source_dir.join("go.mod");
    let go_sum = source_dir.join("go.sum");

    let mut confidence = 0;

    if go_mod.exists() {
        confidence += 70;
    }

    if go_sum.exists() {
        confidence += 10;
    }

    // Look for Go files
    let has_go_files = WalkDir::new(source_dir)
        .max_depth(2)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .any(|entry| {
            entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "go")
        });

    if has_go_files {
        confidence += 20;
    }

    if confidence < 50 {
        return Err(BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
        .into());
    }

    Ok(DetectionResult {
        build_system: "go".to_string(),
        build_function: "go".to_string(),
        build_args: Vec::new(),
        needs_network: true, // Go needs network for dependencies
        confidence,
    })
}

/// Detect Node.js build system
async fn detect_nodejs(source_dir: &Path) -> Result<DetectionResult> {
    let package_json = source_dir.join("package.json");
    if !package_json.exists() {
        return Err(BuildError::NoBuildSystemDetected {
            path: source_dir.display().to_string(),
        }
        .into());
    }

    let mut confidence = 60;

    // Check for common Node.js build indicators
    if let Ok(contents) = tokio::fs::read_to_string(&package_json).await {
        if contents.contains("\"scripts\"") {
            confidence += 15;
        }
        if contents.contains("\"build\"") {
            confidence += 10;
        }
        if contents.contains("\"dependencies\"") {
            confidence += 10;
        }
    }

    // Look for package-lock.json or yarn.lock
    if source_dir.join("package-lock.json").exists() {
        confidence += 5;
    }
    if source_dir.join("yarn.lock").exists() {
        confidence += 5;
    }

    Ok(DetectionResult {
        build_system: "nodejs".to_string(),
        build_function: "nodejs".to_string(),
        build_args: Vec::new(),
        needs_network: true, // Node.js needs network for npm/yarn
        confidence,
    })
}
