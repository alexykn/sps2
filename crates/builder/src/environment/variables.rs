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
            "deps_prefix".to_string(),
            self.deps_prefix.display().to_string(),
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
        self.env_vars
            .insert("PREFIX".to_string(), "/opt/pm/live".to_string());

        // Set BUILD_PREFIX to package-specific prefix (e.g., /hello-1.0.0)
        // This is used for staging directory structure, not for build system --prefix arguments
        // Build systems now use LIVE_PREFIX (/opt/pm/live) for --prefix arguments
        self.env_vars.insert(
            "BUILD_PREFIX".to_string(),
            format!("/{}-{}", self.context.name, self.context.version),
        );

        // Number of parallel jobs
        self.env_vars
            .insert("JOBS".to_string(), Self::cpu_count().to_string());
        self.env_vars
            .insert("MAKEFLAGS".to_string(), format!("-j{}", Self::cpu_count()));

        // Compiler flags for dependency isolation
        // Use placeholder prefix for build paths to enable relocatable packages
        let placeholder_deps = format!("{}/deps", crate::BUILD_PLACEHOLDER_PREFIX);
        self.env_vars.insert(
            "CFLAGS".to_string(),
            format!("-I{placeholder_deps}/include"),
        );
        self.env_vars.insert(
            "CPPFLAGS".to_string(),
            format!("-I{placeholder_deps}/include"),
        );
        self.env_vars
            .insert("LDFLAGS".to_string(), format!("-L{placeholder_deps}/lib"));

        // Prevent system library contamination
        // LIBRARY_PATH is used by compiler/linker at build time
        self.env_vars.insert(
            "LIBRARY_PATH".to_string(),
            format!("{}/lib", placeholder_deps),
        );
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
        let path_components = ["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/opt/pm/live/bin"];

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

    /// Setup environment for build dependencies with proper isolation
    pub(crate) fn setup_build_deps_environment(&mut self) {
        let deps_prefix_display = self.deps_prefix.display();
        let deps_bin = format!("{deps_prefix_display}/bin");
        let deps_lib = format!("{deps_prefix_display}/lib");
        let deps_pkgconfig = format!("{deps_prefix_display}/lib/pkgconfig");
        let deps_share = format!("{deps_prefix_display}/share");

        // Insert build deps right after system paths but before /opt/pm/live/bin
        // This ensures system tools take precedence, then deps, then installed packages
        let current_path = self.env_vars.get("PATH").cloned().unwrap_or_default();
        let path_parts: Vec<&str> = current_path.split(':').collect();

        // Find where system paths end (after /sbin)
        let mut system_end_idx = 0;
        for (i, part) in path_parts.iter().enumerate() {
            if *part == "/sbin" {
                system_end_idx = i + 1;
                break;
            }
        }

        // Build new PATH: system paths, deps, then everything else
        let mut new_path_components = Vec::new();

        // Add system paths
        new_path_components.extend_from_slice(&path_parts[..system_end_idx]);

        // Add deps bin
        new_path_components.push(&deps_bin);

        // Add remaining paths (/opt/pm/live/bin, etc.)
        if system_end_idx < path_parts.len() {
            new_path_components.extend_from_slice(&path_parts[system_end_idx..]);
        }

        let new_path = new_path_components.join(":");
        self.env_vars.insert("PATH".to_string(), new_path);

        // PKG_CONFIG_PATH for dependency discovery
        self.env_vars
            .insert("PKG_CONFIG_PATH".to_string(), deps_pkgconfig.clone());

        // CMAKE_PREFIX_PATH for CMake-based builds
        self.env_vars.insert(
            "CMAKE_PREFIX_PATH".to_string(),
            self.deps_prefix.display().to_string(),
        );

        // Update compiler flags to include build dep paths
        // Use placeholder paths to keep packages relocatable
        let placeholder_deps = format!("{}/deps", crate::BUILD_PLACEHOLDER_PREFIX);
        let placeholder_include = format!("{}/include", placeholder_deps);
        let placeholder_lib = format!("{}/lib", placeholder_deps);

        let current_cflags = self.env_vars.get("CFLAGS").cloned().unwrap_or_default();
        let new_cflags = if current_cflags.is_empty() {
            format!("-I{placeholder_include}")
        } else {
            format!("{current_cflags} -I{placeholder_include}")
        };
        self.env_vars.insert("CFLAGS".to_string(), new_cflags);

        let current_cppflags = self.env_vars.get("CPPFLAGS").cloned().unwrap_or_default();
        let new_cppflags = if current_cppflags.is_empty() {
            format!("-I{placeholder_include}")
        } else {
            format!("{current_cppflags} -I{placeholder_include}")
        };
        self.env_vars.insert("CPPFLAGS".to_string(), new_cppflags);

        let current_ldflags = self.env_vars.get("LDFLAGS").cloned().unwrap_or_default();
        let new_ldflags = if current_ldflags.is_empty() {
            format!("-L{placeholder_lib}")
        } else {
            format!("{current_ldflags} -L{placeholder_lib}")
        };
        self.env_vars.insert("LDFLAGS".to_string(), new_ldflags);

        // Autotools-specific paths
        self.env_vars
            .insert("ACLOCAL_PATH".to_string(), format!("{deps_share}/aclocal"));

        // Ensure library search paths are set for build time
        // LIBRARY_PATH is used by compiler/linker at build time
        self.env_vars.insert("LIBRARY_PATH".to_string(), deps_lib);
        // Note: We don't set LD_LIBRARY_PATH or DYLD_LIBRARY_PATH as they're
        // considered dangerous for isolation and are runtime variables, not build-time
    }
}
