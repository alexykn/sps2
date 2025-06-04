//! Modern Starlark API for sps2 package recipes
//!
//! This module provides a clean, modular API for Starlark recipes using
//! the `#[starlark_module]` macro approach for proper argument handling.

pub mod build_systems;
pub mod context;
pub mod cross;
pub mod features;
pub mod parallel;

use crate::error_helpers::format_metadata_error;
use crate::recipe::RecipeMetadata;
use sps2_errors::Error;
use starlark::environment::GlobalsBuilder;
use starlark::values::list::ListRef;
use starlark::values::Value;

// Re-export main types
pub use context::{BuildContext, BuildExecutor};

/// Parse metadata from Starlark value (dict)
pub fn parse_metadata(value: Value) -> Result<RecipeMetadata, Error> {
    use starlark::values::dict::DictRef;

    let mut metadata = RecipeMetadata::default();

    // Try to get the dict ref from the value
    let dict = DictRef::from_value(value).ok_or_else(|| {
        format_metadata_error(
            "type",
            &format!(
                "metadata() must return a dictionary, got: {}",
                value.get_type()
            ),
        )
    })?;

    // Helper function to get string value from dict with type validation
    let get_string_value = |key: &str| -> Result<Option<String>, Error> {
        match dict.get_str(key) {
            Some(val) => {
                if let Some(s) = val.unpack_str() {
                    Ok(Some(s.to_string()))
                } else {
                    Err(format_metadata_error(
                        key,
                        &format!(
                            "metadata field '{}' must be a string, got: {}",
                            key,
                            val.get_type()
                        ),
                    )
                    .into())
                }
            }
            None => Ok(None),
        }
    };

    // Extract name (required)
    metadata.name = get_string_value("name")?
        .ok_or_else(|| format_metadata_error("name", "metadata must include 'name' field"))?;

    if metadata.name.trim().is_empty() {
        return Err(format_metadata_error("name", "metadata 'name' cannot be empty").into());
    }

    // Extract version (required)
    metadata.version = get_string_value("version")?
        .ok_or_else(|| format_metadata_error("version", "metadata must include 'version' field"))?;

    if metadata.version.trim().is_empty() {
        return Err(format_metadata_error("version", "metadata 'version' cannot be empty").into());
    }

    // Extract optional fields
    metadata.description = get_string_value("description")?;
    metadata.homepage = get_string_value("homepage")?;
    metadata.license = get_string_value("license")?;

    // Extract dependency arrays
    metadata.runtime_deps = get_string_list(&dict, "depends")?;
    metadata.build_deps = get_string_list(&dict, "build_depends")?;

    Ok(metadata)
}

/// Helper function to extract a list of strings from a Starlark dict
fn get_string_list(
    dict: &starlark::values::dict::DictRef,
    key: &str,
) -> Result<Vec<String>, Error> {
    match dict.get_str(key) {
        Some(val) => {
            // Try to get as a list
            if let Some(list) = ListRef::from_value(val) {
                let mut result = Vec::new();
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        result.push(s.to_string());
                    } else {
                        return Err(format_metadata_error(
                            key,
                            &format!(
                                "dependency list '{}' must contain only strings, got: {}",
                                key,
                                item.get_type()
                            ),
                        )
                        .into());
                    }
                }
                Ok(result)
            } else {
                Err(format_metadata_error(
                    key,
                    &format!(
                        "metadata field '{}' must be a list, got: {}",
                        key,
                        val.get_type()
                    ),
                )
                .into())
            }
        }
        None => Ok(Vec::new()), // Field is optional, return empty list
    }
}

/// Register all Starlark API globals
pub fn register_globals(builder: &mut GlobalsBuilder) {
    // Register context functions
    crate::starlark::context::build_context_functions(builder);

    // The actual registration happens in each module
    build_systems::register_globals(builder);
    features::register_globals(builder);
    parallel::register_globals(builder);
    cross::register_globals(builder);
}
