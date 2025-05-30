//! Helper functions for comprehensive error messages

use spsv2_errors::BuildError;
use std::fmt::Write;

/// Format a Starlark parse error with helpful context
pub fn format_parse_error(path: &str, error: &str) -> BuildError {
    let mut message = format!("Failed to parse recipe at '{path}':\n");
    let _ = writeln!(message, "  {error}");

    // Add helpful hints based on common parse errors
    if error.contains("unexpected EOF") || error.contains("unclosed") {
        message.push_str("\nHint: Check for unclosed strings, parentheses, or brackets.");
    } else if error.contains("indentation") {
        message.push_str("\nHint: Starlark uses Python-style indentation. Check that your indentation is consistent.");
    } else if error.contains("def ") {
        message.push_str("\nHint: Function definitions should be at the top level, not indented.");
    }

    BuildError::RecipeError { message }
}

/// Format a missing function error with example
pub fn format_missing_function_error(function_name: &str) -> BuildError {
    let example = match function_name {
        "metadata" => {
            r#"
def metadata():
    return {
        "name": "package-name",
        "version": "1.0.0",
        "description": "Package description",
        "homepage": "https://example.com",
        "license": "MIT"
    }"#
        }
        "build" => {
            r"
def build(ctx):
    # Access context variables:
    # ctx.PREFIX - installation prefix
    # ctx.JOBS - number of parallel jobs
    # ctx.NAME - package name from metadata
    # ctx.VERSION - package version from metadata
    pass"
        }
        _ => "",
    };

    BuildError::RecipeError {
        message: format!("Recipe must define a {function_name} function.\n\nExample:\n{example}"),
    }
}

/// Format metadata validation errors with helpful examples
pub fn format_metadata_error(field: &str, issue: &str) -> BuildError {
    let mut message = format!("Invalid metadata: {issue}\n");

    // Add field-specific help
    match field {
        "name" => {
            message.push_str(
                "\nThe 'name' field must be a non-empty string containing the package name.",
            );
            message.push_str("\nExample: \"name\": \"curl\"");
        }
        "version" => {
            message.push_str(
                "\nThe 'version' field must be a non-empty string containing the package version.",
            );
            message.push_str("\nExample: \"version\": \"8.5.0\"");
        }
        "type" => {
            message
                .push_str("\nThe metadata() function must return a dictionary with string values.");
            message.push_str("\nExample:\n");
            message.push_str("def metadata():\n");
            message.push_str("    return {\n");
            message.push_str("        \"name\": \"mypackage\",\n");
            message.push_str("        \"version\": \"1.0.0\"\n");
            message.push_str("    }");
        }
        _ => {}
    }

    BuildError::RecipeError { message }
}

/// Format build function errors with context
pub fn format_build_error(error: &str) -> BuildError {
    let mut message = format!("Build function failed: {error}\n");

    // Add common troubleshooting tips
    if error.contains("NAME") || error.contains("VERSION") {
        message.push_str(
            "\nHint: Context attributes are uppercase: ctx.NAME, ctx.VERSION, ctx.PREFIX, ctx.JOBS",
        );
    } else if error.contains("ctx") {
        message.push_str("\nHint: The build function receives a context parameter. Make sure your function signature is: def build(ctx):");
    } else if error.contains("method") || error.contains("function") {
        message.push_str("\nNote: Build methods (fetch, make, install) are not yet implemented in the Starlark API.");
    }

    BuildError::RecipeError { message }
}

/// Format recipe evaluation errors with line context if available
pub fn format_eval_error(error: &str) -> BuildError {
    let mut message = String::from("Recipe evaluation failed:\n");

    // Try to extract line number if present
    if let Some(line_start) = error.find("line ") {
        if let Some(line_end) = error[line_start..].find(|c: char| !c.is_numeric() && c != ' ') {
            let line_info = &error[line_start..line_start + line_end];
            let _ = write!(message, "  At {line_info}: ");
        }
    }

    let _ = writeln!(message, "  {error}");

    // Add common fix suggestions
    if error.contains("not defined") || error.contains("unknown") {
        message.push_str("\nHint: Check that all variables and functions are defined before use.");
    } else if error.contains("type") {
        message.push_str(
            "\nHint: Check that you're using the correct types. Starlark is strongly typed.",
        );
    }

    BuildError::RecipeError { message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_parse_error_with_eof_hint() {
        let error = format_parse_error("test.star", "unexpected EOF while parsing");
        let msg = error.to_string();
        assert!(msg.contains("test.star"));
        assert!(msg.contains("Check for unclosed"));
    }

    #[test]
    fn test_format_missing_function_with_example() {
        let error = format_missing_function_error("metadata");
        let msg = error.to_string();
        assert!(msg.contains("def metadata():"));
        assert!(msg.contains("return {"));
    }

    #[test]
    fn test_format_metadata_error_with_example() {
        let error = format_metadata_error("name", "name field is required");
        let msg = error.to_string();
        assert!(msg.contains("non-empty string"));
        assert!(msg.contains("Example:"));
    }
}
