//! Shell command parser for security validation
//!
//! This module implements a simplified shell parser that understands
//! common shell constructs for security validation purposes.

use super::rules::BUILD_VARIABLES;
use sps2_errors::{BuildError, Error};

/// Represents a parsed shell token
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Command(String),
    Argument(String),
    Variable(String),
    Operator(String),
    Redirect(String),
    Quote(char),
    Comment(String),
}

/// Parse a shell command into tokens
pub fn tokenize_shell(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let Some(&ch) = chars.get(i) else {
            break;
        };

        match ch {
            // Skip whitespace (but not newlines - they're command separators)
            ' ' | '\t' | '\r' => i += 1,
            // Newlines act as command separators
            '\n' => {
                tokens.push(Token::Operator("\n".to_string()));
                i += 1;
            }
            // Comments
            '#' => i = parse_comment(&chars, i, &mut tokens),
            // Quotes and backticks (command substitution)
            '"' | '\'' | '`' => i = parse_quoted(&chars, i, ch, &mut tokens),
            // Variables
            '$' => i = parse_variable(&chars, i, &mut tokens),
            // Operators
            ';' => {
                tokens.push(Token::Operator(";".to_string()));
                i += 1;
            }
            '&' => i = parse_ampersand(&chars, i, &mut tokens),
            '|' => i = parse_pipe(&chars, i, &mut tokens),
            '>' | '<' => i = parse_redirect(&chars, i, &mut tokens),
            // Regular tokens
            _ => i = parse_word(&chars, i, &mut tokens),
        }
    }

    tokens
}

/// Parse a comment until end of line
fn parse_comment(chars: &[char], mut i: usize, tokens: &mut Vec<Token>) -> usize {
    let start = i;
    while i < chars.len() && chars.get(i).copied() != Some('\n') {
        i += 1;
    }
    tokens.push(Token::Comment(chars[start..i].iter().collect()));
    i
}

/// Parse a quoted string
fn parse_quoted(chars: &[char], mut i: usize, quote_char: char, tokens: &mut Vec<Token>) -> usize {
    tokens.push(Token::Quote(quote_char));
    i += 1;
    let start = i;
    while i < chars.len() && chars.get(i).copied() != Some(quote_char) {
        if chars.get(i).copied() == Some('\\') && i + 1 < chars.len() {
            i += 2; // Skip escaped character
        } else {
            i += 1;
        }
    }
    if start < i {
        let content = chars[start..i].iter().collect::<String>();
        // For backticks, parse the content as a command substitution
        if quote_char == '`' {
            // Add a special token to indicate command substitution
            tokens.push(Token::Variable(format!("`{content}`")));
        } else {
            tokens.push(Token::Argument(content));
        }
    }
    if chars.get(i).copied() == Some(quote_char) {
        tokens.push(Token::Quote(quote_char));
        i += 1;
    }
    i
}

/// Parse a variable reference
fn parse_variable(chars: &[char], mut i: usize, tokens: &mut Vec<Token>) -> usize {
    let start = i;
    i += 1;
    if chars.get(i).copied() == Some('{') {
        // ${VAR} style
        i += 1;
        while i < chars.len() && chars.get(i).copied() != Some('}') {
            i += 1;
        }
        if i < chars.len() {
            i += 1; // Skip closing }
        }
    } else {
        // $VAR style
        while i < chars.len()
            && chars
                .get(i)
                .is_some_and(|&c| c.is_alphanumeric() || c == '_')
        {
            i += 1;
        }
    }
    tokens.push(Token::Variable(chars[start..i].iter().collect()));
    i
}

/// Parse & or &&
fn parse_ampersand(chars: &[char], mut i: usize, tokens: &mut Vec<Token>) -> usize {
    if chars.get(i + 1).copied() == Some('&') {
        tokens.push(Token::Operator("&&".to_string()));
        i += 2;
    } else {
        tokens.push(Token::Operator("&".to_string()));
        i += 1;
    }
    i
}

/// Parse | or ||
fn parse_pipe(chars: &[char], mut i: usize, tokens: &mut Vec<Token>) -> usize {
    if chars.get(i + 1).copied() == Some('|') {
        tokens.push(Token::Operator("||".to_string()));
        i += 2;
    } else {
        tokens.push(Token::Operator("|".to_string()));
        i += 1;
    }
    i
}

/// Parse redirections
fn parse_redirect(chars: &[char], mut i: usize, tokens: &mut Vec<Token>) -> usize {
    let start = i;
    i += 1;
    if chars.get(i).copied() == chars.get(start).copied() {
        i += 1; // >> or <<
    }
    tokens.push(Token::Redirect(chars[start..i].iter().collect()));
    i
}

/// Parse a word (command or argument)
fn parse_word(chars: &[char], mut i: usize, tokens: &mut Vec<Token>) -> usize {
    let start = i;
    while i < chars.len() {
        let Some(&current_char) = chars.get(i) else {
            break;
        };
        if current_char.is_whitespace()
            || matches!(
                current_char,
                ';' | '&' | '|' | '>' | '<' | '"' | '\'' | '$' | '#'
            )
        {
            break;
        }
        if chars.get(i).copied() == Some('\\') && i + 1 < chars.len() {
            i += 2; // Skip escaped character
        } else {
            i += 1;
        }
    }
    let text: String = chars[start..i].iter().collect();

    // Determine if this is a command or argument based on context
    let is_command = tokens.is_empty() || matches!(tokens.last(), Some(Token::Operator(_)));

    if is_command {
        tokens.push(Token::Command(text));
    } else {
        tokens.push(Token::Argument(text));
    }
    i
}

/// Validate a tokenized shell command
pub fn validate_tokens(
    tokens: &[Token],
    sps2_config: Option<&sps2_config::Config>,
) -> Result<(), Error> {
    for (i, _) in tokens.iter().enumerate() {
        match &tokens[i] {
            Token::Command(cmd) => {
                // Special handling for specific commands
                match cmd.as_str() {
                    "rm" => validate_rm_tokens(&tokens[i..])?,
                    "cd" => {
                        // Validate cd target
                        if let Some(Token::Argument(path)) = tokens.get(i + 1) {
                            validate_cd_path(path)?;
                        }
                    }
                    _ => {}
                }

                // Check if command is in allowlist (unless it's a path)
                if !cmd.contains('/') {
                    // Use config if available
                    if let Some(config) = sps2_config {
                        if !config.is_command_allowed(cmd) {
                            return Err(BuildError::DangerousCommand {
                                command: cmd.clone(),
                                reason: format!(
                                    "Command '{cmd}' is not in the allowed commands list"
                                ),
                            }
                            .into());
                        }
                    } else {
                        // If no config provided, be conservative and reject unknown commands
                        return Err(BuildError::DangerousCommand {
                            command: cmd.clone(),
                            reason: "No configuration provided for allowed commands".to_string(),
                        }
                        .into());
                    }
                }
            }
            Token::Argument(arg) => {
                // Validate arguments based on the preceding command
                if let Some(Token::Command(cmd)) = tokens.get(i.saturating_sub(1)) {
                    validate_command_argument(cmd, arg)?;
                }
            }
            Token::Variable(var) => {
                // Check for command substitution (backticks)
                if var.starts_with('`') && var.ends_with('`') {
                    // Extract the command inside backticks
                    let inner_cmd = &var[1..var.len() - 1];
                    // Recursively validate the inner command
                    let inner_tokens = tokenize_shell(inner_cmd);
                    validate_tokens(&inner_tokens, sps2_config)?;
                } else if !is_safe_variable(var) {
                    // Variables we don't recognize could be dangerous
                    // but we'll allow them with a warning for now
                }
            }
            Token::Operator(op) => {
                if op.as_str() == "&" {
                    // Background execution not allowed
                    return Err(BuildError::DangerousCommand {
                        command: format!("... {op}"),
                        reason: "Background execution is not allowed in build scripts".to_string(),
                    }
                    .into());
                }
                // Other operators (";", "&&", "||", "|") are ok
            }
            Token::Redirect(_) => {
                // Validate the target of redirections
                if let Some(Token::Argument(target)) = tokens.get(i + 1) {
                    validate_redirect_target(target)?;
                }
            }
            Token::Quote(_) | Token::Comment(_) => {
                // These are ok
            }
        }
    }

    Ok(())
}

/// Validate cd path
fn validate_cd_path(path: &str) -> Result<(), Error> {
    // Note: This is a simplified validation used when SecurityContext is not available.
    // The SecurityContext provides more comprehensive validation with variable expansion.

    // Allow if path contains build variables - will be validated after expansion
    for var in BUILD_VARIABLES {
        if path.contains(var) {
            return Ok(());
        }
    }

    // Allow relative paths and paths within build directory
    if !path.starts_with('/') || path.starts_with("/opt/pm/build/") {
        return Ok(());
    }

    // Block cd to system directories (only if it's an absolute path to a system dir)
    if path == "/"
        || path == "/etc"
        || path.starts_with("/etc/")
        || path == "/usr"
        || path.starts_with("/usr/")
        || path == "/bin"
        || path.starts_with("/bin/")
    {
        return Err(BuildError::DangerousCommand {
            command: format!("cd {path}"),
            reason: "Cannot change to system directories".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validate rm command with its arguments
fn validate_rm_tokens(tokens: &[Token]) -> Result<(), Error> {
    let mut has_r = false;
    let mut has_f = false;

    for token in tokens.iter().skip(1) {
        match token {
            Token::Argument(arg) => {
                // Check for -r/-R and -f flags
                if arg.starts_with('-') && !arg.starts_with("--") {
                    if arg.contains('r') || arg.contains('R') {
                        has_r = true;
                    }
                    if arg.contains('f') {
                        has_f = true;
                    }
                } else {
                    // This is a path argument
                    validate_rm_path(arg)?;
                }
            }
            Token::Operator(_) => break, // End of this command
            _ => {}
        }
    }

    // Block rm -rf
    if has_r && has_f {
        return Err(BuildError::DangerousCommand {
            command: "rm -rf".to_string(),
            reason: "rm -rf is not allowed in build scripts".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validate a path for rm command
fn validate_rm_path(path: &str) -> Result<(), Error> {
    // Allow if it contains build variables
    for var in BUILD_VARIABLES {
        if path.contains(var) {
            return Ok(());
        }
    }

    // Block dangerous paths
    if path == "/" || path == "/*" || path.starts_with("/etc") || path.starts_with("/usr") {
        return Err(BuildError::DangerousCommand {
            command: format!("rm {path}"),
            reason: "Attempting to delete system directories".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Validate command arguments
fn validate_command_argument(cmd: &str, arg: &str) -> Result<(), Error> {
    match cmd {
        "chmod" => {
            // Don't allow chmod on system files
            if arg.starts_with("/etc") || arg.starts_with("/usr") || arg.starts_with("/bin") {
                return Err(BuildError::DangerousCommand {
                    command: format!("{cmd} {arg}"),
                    reason: "Cannot modify permissions on system files".to_string(),
                }
                .into());
            }
        }
        _ => {
            // General validation for paths
            // Allow paths with build variables
            for var in BUILD_VARIABLES {
                if arg.contains(var) {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

/// Validate redirect targets
fn validate_redirect_target(target: &str) -> Result<(), Error> {
    // Allow /dev/null as it's commonly used and safe
    if target == "/dev/null" {
        return Ok(());
    }

    // Block redirects to other system files
    if target.starts_with("/etc") || target.starts_with("/dev") || target.starts_with("/sys") {
        return Err(BuildError::DangerousCommand {
            command: format!("> {target}"),
            reason: "Cannot redirect output to system files".to_string(),
        }
        .into());
    }
    Ok(())
}

/// Check if a variable is safe
fn is_safe_variable(var: &str) -> bool {
    BUILD_VARIABLES.contains(&var) ||
    var == "$?" || // Exit code
    var == "$$" || // Process ID
    var == "$#" || // Argument count
    var == "$@" || // All arguments
    var == "$*" // All arguments as single string
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize_shell("echo hello world");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], Token::Command("echo".to_string()));
        assert_eq!(tokens[1], Token::Argument("hello".to_string()));
        assert_eq!(tokens[2], Token::Argument("world".to_string()));
    }

    #[test]
    fn test_tokenize_with_redirect() {
        let tokens = tokenize_shell("echo test > file.txt");
        println!("Tokens: {tokens:?}");
        assert!(tokens.len() >= 4);
        assert_eq!(tokens[0], Token::Command("echo".to_string()));
        assert_eq!(tokens[1], Token::Argument("test".to_string()));
        assert_eq!(tokens[2], Token::Redirect(">".to_string()));
        assert_eq!(tokens[3], Token::Argument("file.txt".to_string()));
    }

    #[test]
    fn test_tokenize_with_variables() {
        let tokens = tokenize_shell("cd ${DESTDIR}/bin");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], Token::Command("cd".to_string()));
        assert_eq!(tokens[1], Token::Variable("${DESTDIR}".to_string()));
        assert_eq!(tokens[2], Token::Argument("/bin".to_string()));
    }

    #[test]
    fn test_validate_allowed_commands() {
        // Create a test config with allowed commands
        let config = sps2_config::Config::default();

        let tokens = tokenize_shell("make install");
        assert!(validate_tokens(&tokens, Some(&config)).is_ok());

        let tokens = tokenize_shell("./configure --prefix=/opt/pm/live");
        assert!(validate_tokens(&tokens, Some(&config)).is_ok());
    }

    #[test]
    fn test_validate_dangerous_commands() {
        let tokens = tokenize_shell("sudo make install");
        assert!(validate_tokens(&tokens, None).is_err());

        let tokens = tokenize_shell("apt-get install foo");
        assert!(validate_tokens(&tokens, None).is_err());
    }

    #[test]
    fn test_validate_rm_rf() {
        let config = sps2_config::Config::default();

        let tokens = tokenize_shell("rm -rf /tmp/build");
        assert!(validate_tokens(&tokens, Some(&config)).is_err());

        let tokens = tokenize_shell("rm -f file.txt");
        assert!(validate_tokens(&tokens, Some(&config)).is_ok());
    }

    #[test]
    fn test_validate_background_execution() {
        let tokens = tokenize_shell("./daemon &");
        assert!(validate_tokens(&tokens, None).is_err());
    }

    #[test]
    fn test_validate_with_build_variables() {
        let config = sps2_config::Config::default();

        let tokens = tokenize_shell("cd ${DESTDIR}/opt/pm/live/bin");
        assert!(validate_tokens(&tokens, Some(&config)).is_ok());

        let tokens = tokenize_shell("rm ${BUILD_DIR}/temp.o");
        assert!(validate_tokens(&tokens, Some(&config)).is_ok());
    }
}
