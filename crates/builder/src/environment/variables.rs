//! Environment variable setup and isolation

use super::core::BuildEnvironment;
use std::collections::HashMap;

impl BuildEnvironment {
    /// Get a summary of the build environment for debugging
    #[must_use]
    pub fn environment_summary(&self) -> HashMap<String, String> {
        let mut summary = HashMap::new();

        summary.insert(
            "build_prefix".to_string(),
            self.build_prefix.display().to_string(),
        );
        summary.insert(
            "staging_dir".to_string(),
            self.staging_dir.display().to_string(),
        );
        summary.insert("package_name".to_string(), self.context.name.clone());
        summary.insert(
            "package_version".to_string(),
            self.context.version.to_string(),
        );

        // Add key environment variables
        for key in &[
            "PATH",
            "PKG_CONFIG_PATH",
            "CMAKE_PREFIX_PATH",
            "CFLAGS",
            "LDFLAGS",
        ] {
            if let Some(value) = self.env_vars.get(*key) {
                summary.insert((*key).to_string(), value.clone());
            }
        }

        summary
    }

    /// Setup base environment variables for isolated build
    pub(crate) fn setup_environment(&mut self) {
        // Clear potentially harmful environment variables for clean build
        self.setup_clean_environment();

        // Add staging dir to environment (standard autotools DESTDIR)
        self.env_vars.insert(
            "DESTDIR".to_string(),
            self.staging_dir.display().to_string(),
        );

        // Set build prefix to final installation location (not staging dir)
        self.env_vars.insert(
            "PREFIX".to_string(),
            sps2_config::fixed_paths::LIVE_DIR.to_string(),
        );

        // Set BUILD_PREFIX to package-specific prefix (e.g., /hello-1.0.0)
        // This is used for staging directory structure, not for build system --prefix arguments
        // Build systems now use LIVE_PREFIX for --prefix arguments
        self.env_vars.insert(
            "BUILD_PREFIX".to_string(),
            format!("/{}-{}", self.context.name, self.context.version),
        );

        // Number of parallel jobs
        self.env_vars
            .insert("JOBS".to_string(), Self::cpu_count().to_string());
        self.env_vars
            .insert("MAKEFLAGS".to_string(), format!("-j{}", Self::cpu_count()));

        // Compiler flags pointing to /opt/pm/live where dependencies are installed
        let live_include = &format!("{}/include", sps2_config::fixed_paths::LIVE_DIR);
        let live_lib = &format!("{}/lib", sps2_config::fixed_paths::LIVE_DIR);
        self.env_vars
            .insert("CFLAGS".to_string(), format!("-I{live_include}"));
        self.env_vars
            .insert("CPPFLAGS".to_string(), format!("-I{live_include}"));
        // Base LDFLAGS with headerpad for macOS
        let mut ldflags = format!("-L{live_lib}");
        if cfg!(target_os = "macos") {
            ldflags.push_str(" -headerpad_max_install_names");
        }
        self.env_vars.insert("LDFLAGS".to_string(), ldflags);

        // Prevent system library contamination
        // LIBRARY_PATH is used by compiler/linker at build time
        self.env_vars
            .insert("LIBRARY_PATH".to_string(), live_lib.to_string());
        // Note: We don't set LD_LIBRARY_PATH or DYLD_LIBRARY_PATH as they're
        // considered dangerous for isolation and are runtime variables, not build-time

        // macOS specific settings - targeting Apple Silicon Macs (macOS 12.0+)
        self.env_vars
            .insert("MACOSX_DEPLOYMENT_TARGET".to_string(), "12.0".to_string());
    }

    /// Setup a clean environment by removing potentially harmful variables
    fn setup_clean_environment(&mut self) {
        // Keep only essential environment variables
        let essential_vars = vec![
            "PATH", "HOME", "USER", "SHELL", "TERM", "LANG", "LC_ALL", "TMPDIR", "TMP", "TEMP",
        ];

        // Start with a minimal PATH containing only system essentials
        // Then add /opt/pm/live/bin for sps2-installed tools
        let path_components = [
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
            sps2_config::fixed_paths::BIN_DIR,
        ];

        self.env_vars
            .insert("PATH".to_string(), path_components.join(":"));

        // Copy essential variables from host environment (except PATH)
        for var in essential_vars {
            if var != "PATH" {
                if let Ok(value) = std::env::var(var) {
                    self.env_vars.insert(var.to_string(), value);
                }
            }
        }

        // Clear potentially problematic variables
        self.env_vars.remove("CFLAGS");
        self.env_vars.remove("CPPFLAGS");
        self.env_vars.remove("LDFLAGS");
        self.env_vars.remove("PKG_CONFIG_PATH");
        self.env_vars.remove("LIBRARY_PATH");
        self.env_vars.remove("LD_LIBRARY_PATH");
        self.env_vars.remove("DYLD_LIBRARY_PATH");
        self.env_vars.remove("CMAKE_PREFIX_PATH");
        self.env_vars.remove("ACLOCAL_PATH");
    }

    /// Setup environment for build dependencies
    pub(crate) fn setup_build_deps_environment(&mut self) {
        // Since dependencies are installed in /opt/pm/live, we just need to ensure
        // the paths are set correctly. PATH already includes /opt/pm/live/bin

        // PKG_CONFIG_PATH for dependency discovery
        self.env_vars.insert(
            "PKG_CONFIG_PATH".to_string(),
            format!("{}/lib/pkgconfig", sps2_config::fixed_paths::LIVE_DIR),
        );

        // CMAKE_PREFIX_PATH for CMake-based builds
        self.env_vars.insert(
            "CMAKE_PREFIX_PATH".to_string(),
            sps2_config::fixed_paths::LIVE_DIR.to_string(),
        );

        // Autotools-specific paths
        self.env_vars.insert(
            "ACLOCAL_PATH".to_string(),
            format!("{}/share/aclocal", sps2_config::fixed_paths::LIVE_DIR),
        );

        // CFLAGS, LDFLAGS, and LIBRARY_PATH are already set to /opt/pm/live in setup_environment()
    }

    /// Apply default compiler flags for optimization and security
    ///
    /// This method sets recommended compiler flags for macOS ARM64 builds.
    /// It preserves existing flags while adding optimizations.
    /// Does NOT modify dependency paths - those are handled separately.
    pub fn apply_default_compiler_flags(&mut self) {
        // Mark that with_defaults() was called
        self.with_defaults_called = true;
        // Detect target architecture
        let arch = std::env::consts::ARCH;
        let is_arm64 = arch == "aarch64";
        let is_macos = cfg!(target_os = "macos");

        // Base C/C++ optimization flags
        let mut base_cflags = vec![
            "-O2",                      // Standard optimization level
            "-pipe",                    // Use pipes instead of temp files
            "-fstack-protector-strong", // Stack protection for security
        ];

        // Architecture-specific optimizations for Apple Silicon
        if is_arm64 && is_macos {
            // Use apple-m1 as a baseline for all Apple Silicon
            // This is compatible with M1, M2, M3, and newer
            base_cflags.extend(&[
                "-mcpu=apple-m1", // Target Apple Silicon baseline
                "-mtune=native",  // Tune for the build machine
            ]);
        }

        // Merge C flags with existing ones
        self.merge_compiler_flags("CFLAGS", &base_cflags);
        self.merge_compiler_flags("CXXFLAGS", &base_cflags);

        // Linker flags for macOS
        if is_macos {
            let linker_flags = vec![
                "-Wl,-dead_strip",              // Remove unused code
                "-headerpad_max_install_names", // Reserve space for install name changes
            ];
            self.merge_compiler_flags("LDFLAGS", &linker_flags);
        }

        // Rust-specific optimizations
        if is_arm64 && is_macos {
            // Set RUSTFLAGS for cargo builds
            let rust_flags = ["-C", "target-cpu=apple-m1", "-C", "opt-level=2"];
            let rust_flags_str = rust_flags.join(" ");

            if let Some(existing) = self.env_vars.get("RUSTFLAGS") {
                if existing.is_empty() {
                    self.env_vars
                        .insert("RUSTFLAGS".to_string(), rust_flags_str);
                } else {
                    self.env_vars.insert(
                        "RUSTFLAGS".to_string(),
                        format!("{rust_flags_str} {existing}"),
                    );
                }
            } else {
                self.env_vars
                    .insert("RUSTFLAGS".to_string(), rust_flags_str);
            }
        }

        // Go-specific optimizations
        if is_arm64 && is_macos {
            // CGO flags inherit from CFLAGS/LDFLAGS automatically
            // but we can set explicit Go flags
            self.env_vars
                .insert("GOFLAGS".to_string(), "-buildmode=pie".to_string());
        }

        // Python-specific architecture flag
        if is_arm64 && is_macos {
            self.env_vars
                .insert("ARCHFLAGS".to_string(), "-arch arm64".to_string());
        }

        // CMake-specific variables (will be picked up by CMake build system)
        if is_arm64 && is_macos {
            self.env_vars
                .insert("CMAKE_OSX_ARCHITECTURES".to_string(), "arm64".to_string());
        }

        // Note: CMAKE_INSTALL_NAME_DIR is now handled by the CMake build system
        // as a command-line argument when with_defaults() is used
    }

    /// Helper to merge compiler flags without duplicating
    fn merge_compiler_flags(&mut self, var_name: &str, new_flags: &[&str]) {
        let existing = self.env_vars.get(var_name).cloned().unwrap_or_default();

        // Convert new flags to string
        let new_flags_str = new_flags.join(" ");

        // Merge with existing flags
        let merged = if existing.is_empty() {
            new_flags_str
        } else {
            // Prepend optimization flags so user flags can override
            format!("{new_flags_str} {existing}")
        };

        self.env_vars.insert(var_name.to_string(), merged);
    }
}
