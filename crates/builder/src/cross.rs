// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Enhanced cross-compilation infrastructure for sps2
//!
//! This module provides comprehensive cross-compilation support including:
//! - Platform detection and validation
//! - Toolchain management
//! - Sysroot configuration
//! - Build system integration

use crate::build_systems::{CrossCompilationContext, Platform, Toolchain};
use sps2_errors::{BuildError, Error};
use sps2_events::{Event, EventSender};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;

/// Enhanced cross-compilation context with setup methods
pub struct EnhancedCrossContext {
    /// Base cross-compilation context
    pub base: CrossCompilationContext,
    /// Validated toolchain paths
    pub validated_tools: HashMap<String, PathBuf>,
    /// Multiarch library paths
    pub multiarch_paths: Vec<PathBuf>,
    /// Pkg-config paths for target
    pub pkg_config_paths: Vec<PathBuf>,
    /// Additional cross-compilation flags
    pub cross_flags: HashMap<String, String>,
    /// Event sender for progress reporting
    pub event_sender: Option<EventSender>,
}

impl EnhancedCrossContext {
    /// Create a new enhanced cross-compilation context
    ///
    /// # Errors
    ///
    /// Returns an error if platform validation or toolchain detection fails
    pub async fn new(
        build_triple: &str,
        host_triple: &str,
        target_triple: Option<&str>,
        sysroot: PathBuf,
        event_sender: Option<EventSender>,
    ) -> Result<Self, Error> {
        // Parse and validate platforms
        let build_platform = Platform::from_triple(build_triple)?;
        let host_platform = Platform::from_triple(host_triple)?;
        let target_platform = if let Some(triple) = target_triple {
            Some(Platform::from_triple(triple)?)
        } else {
            None
        };

        // Validate platform compatibility
        Self::validate_platform_compatibility(&build_platform, &host_platform)?;

        // Detect toolchain
        let toolchain = Self::detect_toolchain(&host_platform, &sysroot).await?;

        // Create base context
        let base = CrossCompilationContext {
            build_platform,
            host_platform,
            target_platform,
            sysroot: sysroot.clone(),
            toolchain,
        };

        // Create enhanced context
        let mut ctx = Self {
            base,
            validated_tools: HashMap::new(),
            multiarch_paths: Vec::new(),
            pkg_config_paths: Vec::new(),
            cross_flags: HashMap::new(),
            event_sender,
        };

        // Setup the context
        ctx.setup().await?;

        Ok(ctx)
    }

    /// Setup cross-compilation environment
    async fn setup(&mut self) -> Result<(), Error> {
        // Send event
        if let Some(sender) = &self.event_sender {
            let _ = sender.send(Event::BuildStepStarted {
                package: "cross-compilation".to_string(),
                step: "setup".to_string(),
            });
        }

        // Validate toolchain binaries
        self.validate_toolchain().await?;

        // Setup sysroot
        self.setup_sysroot().await?;

        // Setup multiarch paths
        self.setup_multiarch_paths()?;

        // Setup pkg-config
        self.setup_pkg_config()?;

        // Setup cross-compilation flags
        self.setup_cross_flags()?;

        Ok(())
    }

    /// Validate toolchain binaries exist and are executable
    async fn validate_toolchain(&mut self) -> Result<(), Error> {
        let tools = [
            ("cc", &self.base.toolchain.cc),
            ("cxx", &self.base.toolchain.cxx),
            ("ar", &self.base.toolchain.ar),
            ("strip", &self.base.toolchain.strip),
            ("ranlib", &self.base.toolchain.ranlib),
        ];

        for (name, tool) in &tools {
            let path = Self::find_tool(tool).await?;
            self.validated_tools.insert((*name).to_string(), path);
        }

        Ok(())
    }

    /// Find a tool in PATH or as absolute path
    async fn find_tool(tool: &str) -> Result<PathBuf, Error> {
        // If it's already an absolute path, just check it exists
        let path = Path::new(tool);
        if path.is_absolute() {
            if fs::metadata(path).await.is_ok() {
                return Ok(path.to_path_buf());
            }
            return Err(BuildError::Failed {
                message: format!("Tool not found: {tool}"),
            }
            .into());
        }

        // Try to find in PATH
        let output = Command::new("which")
            .arg(tool)
            .output()
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to find tool {tool}: {e}"),
            })?;

        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(PathBuf::from(path_str))
        } else {
            Err(BuildError::Failed {
                message: format!("Tool not found in PATH: {tool}"),
            }
            .into())
        }
    }

    /// Setup and validate sysroot
    async fn setup_sysroot(&mut self) -> Result<(), Error> {
        // Check sysroot exists
        if !self.base.sysroot.exists() {
            return Err(BuildError::Failed {
                message: format!("Sysroot does not exist: {}", self.base.sysroot.display()),
            }
            .into());
        }

        // Validate sysroot structure
        let required_dirs = ["usr/include", "usr/lib", "lib"];
        for dir in &required_dirs {
            let path = self.base.sysroot.join(dir);
            if !path.exists() {
                // Try to create it if missing
                fs::create_dir_all(&path)
                    .await
                    .map_err(|e| BuildError::Failed {
                        message: format!(
                            "Failed to create sysroot directory {}: {e}",
                            path.display()
                        ),
                    })?;
            }
        }

        Ok(())
    }

    /// Setup multiarch library paths
    fn setup_multiarch_paths(&mut self) -> Result<(), Error> {
        let arch_triplet = self.get_multiarch_triplet();

        // Add standard multiarch paths
        let lib_paths = [
            format!("usr/lib/{arch_triplet}"),
            format!("lib/{arch_triplet}"),
            "usr/lib".to_string(),
            "lib".to_string(),
        ];

        for lib_path in &lib_paths {
            let full_path = self.base.sysroot.join(lib_path);
            if full_path.exists() {
                self.multiarch_paths.push(full_path);
            }
        }

        Ok(())
    }

    /// Get multiarch triplet for the target platform
    fn get_multiarch_triplet(&self) -> String {
        match (
            self.base.host_platform.arch.as_str(),
            self.base.host_platform.os.as_str(),
            self.base.host_platform.abi.as_deref(),
        ) {
            ("aarch64", "linux", Some("gnu")) => "aarch64-linux-gnu".to_string(),
            ("aarch64", "linux", Some("musl")) => "aarch64-linux-musl".to_string(),
            ("x86_64", "linux", Some("gnu")) => "x86_64-linux-gnu".to_string(),
            ("x86_64", "linux", Some("musl")) => "x86_64-linux-musl".to_string(),
            ("armv7", "linux", Some("gnueabihf")) => "arm-linux-gnueabihf".to_string(),
            ("armv7", "linux", Some("gnueabi")) => "arm-linux-gnueabi".to_string(),
            _ => self.base.host_platform.triple(),
        }
    }

    /// Setup pkg-config for cross-compilation
    fn setup_pkg_config(&mut self) -> Result<(), Error> {
        // Add pkg-config paths from multiarch directories
        for lib_path in &self.multiarch_paths {
            let pkgconfig_path = lib_path.join("pkgconfig");
            if pkgconfig_path.exists() {
                self.pkg_config_paths.push(pkgconfig_path);
            }
        }

        // Also check standard locations
        let standard_paths = [
            self.base.sysroot.join("usr/share/pkgconfig"),
            self.base.sysroot.join("usr/local/lib/pkgconfig"),
        ];

        for path in &standard_paths {
            if path.exists() {
                self.pkg_config_paths.push(path.clone());
            }
        }

        Ok(())
    }

    /// Setup cross-compilation flags
    fn setup_cross_flags(&mut self) -> Result<(), Error> {
        let arch = &self.base.host_platform.arch;
        let os = &self.base.host_platform.os;

        // Common flags
        self.cross_flags.insert(
            "CFLAGS".to_string(),
            format!("--sysroot={}", self.base.sysroot.display()),
        );
        self.cross_flags.insert(
            "CXXFLAGS".to_string(),
            format!("--sysroot={}", self.base.sysroot.display()),
        );
        // LDFLAGS with sysroot and headerpad for macOS
        let mut ldflags = format!("--sysroot={}", self.base.sysroot.display());
        if self.base.host_platform.os == "darwin" {
            ldflags.push_str(" -headerpad_max_install_names");
        }
        self.cross_flags.insert("LDFLAGS".to_string(), ldflags);

        // Architecture-specific flags
        match arch.as_str() {
            "aarch64" => {
                self.cross_flags
                    .insert("CFLAGS".to_string(), "-march=armv8-a".to_string());
            }
            "armv7" => {
                self.cross_flags
                    .insert("CFLAGS".to_string(), "-march=armv7-a".to_string());
                if self.base.host_platform.abi.as_deref() == Some("gnueabihf") {
                    self.cross_flags
                        .insert("CFLAGS".to_string(), "-mfpu=neon-vfpv4".to_string());
                }
            }
            _ => {}
        }

        // OS-specific flags
        if os == "darwin" {
            // macOS cross-compilation flags
            self.cross_flags
                .insert("MACOSX_DEPLOYMENT_TARGET".to_string(), "11.0".to_string());
        }

        Ok(())
    }

    /// Get all environment variables for cross-compilation
    pub fn get_cross_env_vars(&self) -> HashMap<String, String> {
        let mut vars = self.base.get_cross_env_vars();

        // Add enhanced paths
        if !self.pkg_config_paths.is_empty() {
            let pkg_config_path = self
                .pkg_config_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(":");
            vars.insert("PKG_CONFIG_PATH".to_string(), pkg_config_path);
        }

        // Add library paths
        if !self.multiarch_paths.is_empty() {
            let lib_path = self
                .multiarch_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(":");
            vars.insert("LIBRARY_PATH".to_string(), lib_path.clone());
            vars.insert("LD_LIBRARY_PATH".to_string(), lib_path);
        }

        // Add cross flags
        for (key, value) in &self.cross_flags {
            vars.entry(key.clone())
                .and_modify(|v| v.push_str(&format!(" {value}")))
                .or_insert_with(|| value.clone());
        }

        // Add target triple for Rust
        vars.insert(
            "CARGO_TARGET_DIR".to_string(),
            format!("target/{}", self.base.host_platform.triple()),
        );
        vars.insert(
            "CARGO_BUILD_TARGET".to_string(),
            self.base.host_platform.triple(),
        );

        vars
    }

    /// Generate CMake toolchain file
    pub async fn generate_cmake_toolchain_file(&self) -> Result<PathBuf, Error> {
        let toolchain_file = self.base.toolchain.cmake_toolchain_file.clone();

        // Create directory if needed
        if let Some(parent) = toolchain_file.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| BuildError::Failed {
                    message: format!("Failed to create toolchain directory: {e}"),
                })?;
        }

        let system_name = match self.base.host_platform.os.as_str() {
            "darwin" => "Darwin",
            "linux" => "Linux",
            "windows" => "Windows",
            "freebsd" => "FreeBSD",
            _ => "Generic",
        };

        let content = format!(
            r#"# CMake toolchain file for cross-compilation
# Generated by sps2

set(CMAKE_SYSTEM_NAME {system_name})
set(CMAKE_SYSTEM_PROCESSOR {})

# Sysroot
set(CMAKE_SYSROOT {})
set(CMAKE_STAGING_PREFIX {})

# Compilers
set(CMAKE_C_COMPILER {})
set(CMAKE_CXX_COMPILER {})
set(CMAKE_AR {} CACHE FILEPATH "Archiver")
set(CMAKE_RANLIB {} CACHE FILEPATH "Ranlib")
set(CMAKE_STRIP {} CACHE FILEPATH "Strip")

# Flags
set(CMAKE_C_FLAGS_INIT "{}")
set(CMAKE_CXX_FLAGS_INIT "{}")
set(CMAKE_EXE_LINKER_FLAGS_INIT "{}")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "{}")

# Find root settings
set(CMAKE_FIND_ROOT_PATH {})
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# Library paths
set(CMAKE_LIBRARY_PATH {})
set(CMAKE_INCLUDE_PATH {}/usr/include)

# pkg-config
set(ENV{{PKG_CONFIG_PATH}} "{}")
set(ENV{{PKG_CONFIG_SYSROOT_DIR}} "{}")
"#,
            self.base.host_platform.arch,
            self.base.sysroot.display(),
            self.base.sysroot.display(),
            self.validated_tools.get("cc").unwrap().display(),
            self.validated_tools.get("cxx").unwrap().display(),
            self.validated_tools.get("ar").unwrap().display(),
            self.validated_tools.get("ranlib").unwrap().display(),
            self.validated_tools.get("strip").unwrap().display(),
            self.cross_flags.get("CFLAGS").unwrap_or(&String::new()),
            self.cross_flags.get("CXXFLAGS").unwrap_or(&String::new()),
            self.cross_flags.get("LDFLAGS").unwrap_or(&String::new()),
            self.cross_flags.get("LDFLAGS").unwrap_or(&String::new()),
            self.base.sysroot.display(),
            self.multiarch_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(";"),
            self.base.sysroot.display(),
            self.pkg_config_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(":"),
            self.base.sysroot.display(),
        );

        fs::write(&toolchain_file, content)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to write CMake toolchain file: {e}"),
            })?;

        Ok(toolchain_file)
    }

    /// Generate Meson cross file
    pub async fn generate_meson_cross_file(&self) -> Result<PathBuf, Error> {
        let cross_file = self.base.toolchain.meson_cross_file.clone();

        // Create directory if needed
        if let Some(parent) = cross_file.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| BuildError::Failed {
                    message: format!("Failed to create cross file directory: {e}"),
                })?;
        }

        let cpu_family = match self.base.host_platform.arch.as_str() {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            "armv7" => "arm",
            "i686" => "x86",
            _ => &self.base.host_platform.arch,
        };

        let content = format!(
            r"# Meson cross file for cross-compilation
# Generated by sps2

[binaries]
c = '{}'
cpp = '{}'
ar = '{}'
strip = '{}'
ranlib = '{}'
pkg-config = 'pkg-config'

[properties]
sys_root = '{}'
pkg_config_libdir = '{}'

[built-in options]
c_args = [{}]
cpp_args = [{}]
c_link_args = [{}]
cpp_link_args = [{}]

[host_machine]
system = '{}'
cpu_family = '{}'
cpu = '{}'
endian = '{}'
",
            self.validated_tools.get("cc").unwrap().display(),
            self.validated_tools.get("cxx").unwrap().display(),
            self.validated_tools.get("ar").unwrap().display(),
            self.validated_tools.get("strip").unwrap().display(),
            self.validated_tools.get("ranlib").unwrap().display(),
            self.base.sysroot.display(),
            self.pkg_config_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(":"),
            self.format_meson_args(self.cross_flags.get("CFLAGS").unwrap_or(&String::new())),
            self.format_meson_args(self.cross_flags.get("CXXFLAGS").unwrap_or(&String::new())),
            self.format_meson_args(self.cross_flags.get("LDFLAGS").unwrap_or(&String::new())),
            self.format_meson_args(self.cross_flags.get("LDFLAGS").unwrap_or(&String::new())),
            self.base.host_platform.os,
            cpu_family,
            self.base.host_platform.arch,
            self.get_endianness(),
        );

        fs::write(&cross_file, content)
            .await
            .map_err(|e| BuildError::Failed {
                message: format!("Failed to write Meson cross file: {e}"),
            })?;

        Ok(cross_file)
    }

    /// Format arguments for Meson arrays
    fn format_meson_args(&self, args: &str) -> String {
        args.split_whitespace()
            .map(|arg| format!("'{arg}'"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Get endianness for the target architecture
    fn get_endianness(&self) -> &'static str {
        match self.base.host_platform.arch.as_str() {
            "x86_64" | "i686" | "aarch64" => "little",
            "powerpc" | "sparc" => "big",
            _ => "little", // Default to little endian
        }
    }

    /// Generate cargo configuration for cross-compilation
    pub async fn generate_cargo_config(&self) -> Result<String, Error> {
        let target = self.base.host_platform.triple();
        let linker = self.validated_tools.get("cc").unwrap();

        let config = format!(
            r#"# Cargo configuration for cross-compilation
# Generated by sps2

[target.{}]
linker = "{}"
ar = "{}"
rustflags = ["-C", "link-arg=--sysroot={}", "-C", "target-feature=+crt-static"]

[env]
CARGO_TARGET_DIR = "target/{}"
"#,
            target,
            linker.display(),
            self.validated_tools.get("ar").unwrap().display(),
            self.base.sysroot.display(),
            target,
        );

        Ok(config)
    }

    /// Configure autotools for cross-compilation
    pub fn get_autotools_configure_args(&self) -> Vec<String> {
        let mut args = vec![
            format!("--host={}", self.base.host_platform.triple()),
            format!("--build={}", self.base.build_platform.triple()),
        ];

        if let Some(target) = &self.base.target_platform {
            args.push(format!("--target={}", target.triple()));
        }

        // Add sysroot to flags
        args.push(format!(
            "CFLAGS=--sysroot={} {}",
            self.base.sysroot.display(),
            self.cross_flags.get("CFLAGS").unwrap_or(&String::new())
        ));
        args.push(format!(
            "CXXFLAGS=--sysroot={} {}",
            self.base.sysroot.display(),
            self.cross_flags.get("CXXFLAGS").unwrap_or(&String::new())
        ));
        args.push(format!(
            "LDFLAGS=--sysroot={} {}",
            self.base.sysroot.display(),
            self.cross_flags.get("LDFLAGS").unwrap_or(&String::new())
        ));

        args
    }

    /// Validate platform compatibility
    fn validate_platform_compatibility(build: &Platform, host: &Platform) -> Result<(), Error> {
        // Check if cross-compilation is actually needed
        if build.triple() == host.triple() {
            return Err(BuildError::Failed {
                message: "Build and host platforms are the same, cross-compilation not needed"
                    .to_string(),
            }
            .into());
        }

        // Validate OS compatibility
        match (build.os.as_str(), host.os.as_str()) {
            ("linux", "linux") => Ok(()),
            ("darwin", "darwin") => Ok(()),
            ("linux", "windows") => Ok(()), // Linux can cross-compile to Windows
            ("darwin", "linux") => Ok(()),  // macOS can cross-compile to Linux
            ("linux", "darwin") => Err(BuildError::Failed {
                message: "Cross-compilation from Linux to macOS is not supported".to_string(),
            }
            .into()),
            _ => Err(BuildError::Failed {
                message: format!(
                    "Cross-compilation from {} to {} is not supported",
                    build.os, host.os
                ),
            }
            .into()),
        }
    }

    /// Detect toolchain for the target platform
    async fn detect_toolchain(platform: &Platform, sysroot: &Path) -> Result<Toolchain, Error> {
        let prefix = Self::get_toolchain_prefix(platform);

        // Try common toolchain locations
        let possible_prefixes = if prefix.is_empty() {
            vec![String::new()]
        } else {
            vec![
                format!("{prefix}-"),
                format!("{}-", platform.triple()),
                String::new(),
            ]
        };

        let mut toolchain = Toolchain {
            cc: String::new(),
            cxx: String::new(),
            ar: String::new(),
            strip: String::new(),
            ranlib: String::new(),
            cmake_toolchain_file: sysroot.join("toolchain.cmake"),
            meson_cross_file: sysroot.join("cross-file.ini"),
        };

        // Find C compiler
        for prefix in &possible_prefixes {
            let cc = format!("{prefix}gcc");
            if Self::find_tool(&cc).await.is_ok() {
                toolchain.cc = cc;
                toolchain.cxx = format!("{prefix}g++");
                toolchain.ar = format!("{prefix}ar");
                toolchain.strip = format!("{prefix}strip");
                toolchain.ranlib = format!("{prefix}ranlib");
                break;
            }

            let cc = format!("{prefix}clang");
            if Self::find_tool(&cc).await.is_ok() {
                toolchain.cc = cc;
                toolchain.cxx = format!("{prefix}clang++");
                toolchain.ar = format!("{prefix}llvm-ar");
                toolchain.strip = format!("{prefix}llvm-strip");
                toolchain.ranlib = format!("{prefix}llvm-ranlib");
                break;
            }
        }

        if toolchain.cc.is_empty() {
            return Err(BuildError::Failed {
                message: format!("No suitable toolchain found for {}", platform.triple()),
            }
            .into());
        }

        Ok(toolchain)
    }

    /// Get toolchain prefix for common platforms
    fn get_toolchain_prefix(platform: &Platform) -> String {
        match (
            platform.arch.as_str(),
            platform.os.as_str(),
            platform.abi.as_deref(),
        ) {
            ("aarch64", "linux", Some("gnu")) => "aarch64-linux-gnu",
            ("aarch64", "linux", Some("musl")) => "aarch64-linux-musl",
            ("x86_64", "linux", Some("gnu")) => "x86_64-linux-gnu",
            ("x86_64", "linux", Some("musl")) => "x86_64-linux-musl",
            ("armv7", "linux", Some("gnueabihf")) => "arm-linux-gnueabihf",
            ("armv7", "linux", Some("gnueabi")) => "arm-linux-gnueabi",
            ("x86_64", "windows", Some("gnu")) => "x86_64-w64-mingw32",
            ("i686", "windows", Some("gnu")) => "i686-w64-mingw32",
            _ => "",
        }
        .to_string()
    }
}

impl Platform {
    /// Parse a target triple into a Platform
    ///
    /// # Errors
    ///
    /// Returns an error if the triple is malformed
    pub fn from_triple(triple: &str) -> Result<Self, Error> {
        let parts: Vec<&str> = triple.split('-').collect();

        if parts.len() < 2 {
            return Err(BuildError::Failed {
                message: format!("Invalid target triple: {triple}"),
            }
            .into());
        }

        let arch = parts[0].to_string();

        // Handle different triple formats
        let (vendor, os, abi) = match parts.len() {
            2 => {
                // arch-os format (e.g., aarch64-linux)
                (None, parts[1].to_string(), None)
            }
            3 => {
                // Could be arch-vendor-os or arch-os-abi
                if Self::is_vendor(parts[1]) {
                    (Some(parts[1].to_string()), parts[2].to_string(), None)
                } else {
                    (None, parts[1].to_string(), Some(parts[2].to_string()))
                }
            }
            4 => {
                // arch-vendor-os-abi format
                (
                    Some(parts[1].to_string()),
                    parts[2].to_string(),
                    Some(parts[3].to_string()),
                )
            }
            _ => {
                // Handle longer formats by taking last part as ABI
                (
                    Some(parts[1].to_string()),
                    parts[2].to_string(),
                    Some(parts[3..].join("-")),
                )
            }
        };

        Ok(Self {
            arch,
            os,
            abi,
            vendor,
        })
    }

    /// Check if a string is likely a vendor name
    fn is_vendor(s: &str) -> bool {
        matches!(s, "apple" | "pc" | "unknown" | "none" | "w64")
    }
}
