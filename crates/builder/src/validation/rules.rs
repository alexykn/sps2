//! Security rules and patterns for command validation

// Note: Command validation is now done via config.toml allowlist.
// Only commands explicitly listed in ~/.config/sps2/config.toml are allowed.

/// Build-time variables that are safe to use in paths
pub const BUILD_VARIABLES: &[&str] = &[
    "${DESTDIR}",
    "${PREFIX}",
    "${BUILD_DIR}",
    "${SOURCE_DIR}",
    "${srcdir}",
    "${builddir}",
    "${pkgdir}",
    "${PWD}",
    "${OLDPWD}",
];

/// Dangerous patterns to check for in shell commands
pub const DANGEROUS_PATTERNS: &[&str] = &[
    // Attempts to modify shell profile
    "~/.bashrc",
    "~/.profile",
    "~/.zshrc",
    "/etc/profile",
    // Attempts to modify system configs
    "/etc/passwd",
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/hosts",
    // Fork bombs
    ":(){ :|:& };:",
    // Attempts to redirect to system files
    "> /etc/",
    ">> /etc/",
    "> /sys/",
    ">> /sys/",
    "> /dev/",
    ">> /dev/",
    // Attempts to read sensitive files
    "/etc/shadow",
    "/private/etc/", // macOS system files
    "~/.ssh/",
    // Background process attempts
    "nohup",
    "disown",
    "&>/dev/null &", // Running in background
    // Dangerous environment modifications
    "export PATH=",
    "export LD_LIBRARY_PATH=",
    "export DYLD_", // macOS dynamic linker
];

/// System paths that should not be modified
pub const SYSTEM_PATHS: &[&str] = &[
    "/",
    "/bin",
    "/sbin",
    "/etc",
    "/sys",
    "/proc",
    "/dev",
    "/boot",
    "/lib",
    "/lib64",
    "/usr/bin",
    "/usr/sbin",
    "/usr/lib",
    "/usr/include", // Ok to read, not to write
    "/usr/local",   // Ok to read, careful with writes
    "/var",
    "/tmp", // Be careful - some ops might be ok
    "/root",
    "/home", // User homes should not be touched
    // macOS specific
    "/System",
    "/Library",
    "/Applications",
    "/Users",
    "/private",
    "/cores",
    "/Network",
    "/Volumes",
    // Our own paths that should not be modified directly
    "/opt/pm/state", // State database
    "/opt/pm/index", // Package index
    "/opt/pm/live",  // Live packages - only through proper APIs
];

/// Check if a command is trying to use rsync remotely
pub fn is_remote_rsync(args: &[String]) -> bool {
    // Look for remote rsync patterns like user@host: or host:
    args.iter().any(|arg| {
        arg.contains('@') && arg.contains(':') || // user@host:path
        arg.matches(':').count() == 1 && !arg.starts_with('/') // host:path
    })
}

/// Check if a path is within the build environment
pub fn is_within_build_env(path: &str) -> bool {
    // Allowed paths during build
    path.starts_with("/opt/pm/build/") ||
    path.starts_with("./") ||
    path.starts_with("../") && !path.contains("../../..") || // Max 2 levels up
    !path.starts_with('/') // Relative paths are ok
}

/// Check if a URL is suspicious
pub fn is_suspicious_url(url: &str) -> bool {
    // Check for data exfiltration attempts
    url.contains("webhook") ||
    url.contains("requestbin") ||
    url.contains("ngrok.io") ||
    url.contains("localhost") ||
    url.contains("127.0.0.1") ||
    url.contains("0.0.0.0") ||

    // Check for non-standard ports that might indicate C&C
    url.contains(":1337") ||
    url.contains(":31337") ||
    url.contains(":4444") ||
    url.contains(":8888")
}
