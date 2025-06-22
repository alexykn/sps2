//! Security rules and patterns for command validation

/// Commands that should never be allowed during builds
pub const DANGEROUS_COMMANDS: &[&str] = &[
    // Privilege escalation
    "sudo",
    "doas",
    "su",
    "pkexec",
    "gksudo",
    "kdesudo",
    // System modification
    "systemctl",
    "service",
    "init",
    "launchctl",
    "update-rc.d",
    "chkconfig",
    // User management
    "useradd",
    "usermod",
    "userdel",
    "groupadd",
    "groupmod",
    "groupdel",
    "passwd",
    "chpasswd",
    // Package management (builds shouldn't install system packages)
    "apt",
    "apt-get",
    "yum",
    "dnf",
    "pacman",
    "zypper",
    "brew", // Even homebrew should not be used during builds
    // Dangerous file operations
    "shred",
    "dd",   // Can overwrite devices
    "mkfs", // Can format filesystems
    // Network operations that could exfiltrate data
    "nc", // netcat
    "netcat",
    "ncat",
    "telnet",
    "ssh", // No SSH during builds
    "scp",
    // System shutdown/reboot
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    // Kernel/module management
    "modprobe",
    "insmod",
    "rmmod",
    "lsmod",
    // Resource limits that could affect system
    "ulimit", // Could be used to bypass limits
    "nice",   // Could affect system performance
    "renice",
    "ionice",
    // Container escapes
    "nsenter",
    "docker",
    "podman",
    "chroot", // Could be used to escape
];

// Note: ALLOWED_COMMANDS is now deprecated in favor of config.toml
// This is kept only for backwards compatibility when config is not available

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
