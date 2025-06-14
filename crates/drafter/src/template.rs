//! Template rendering for recipe generation

use crate::{BuildInfo, RecipeMetadata, Result, SourceLocation};
use serde::Serialize;
use sps2_errors::BuildError;
use tera::{Context, Tera};

/// Template context for rendering
#[derive(Serialize)]
struct TemplateContext {
    // Basic metadata
    name: String,
    version: String,
    description: Option<String>,
    license: Option<String>,
    homepage: Option<String>,

    // Source information
    source_url: Option<String>,
    source_hash: Option<String>,
    is_git_source: bool,
    git_ref: Option<String>,

    // Build system information
    build_system: String,
    build_function: String,
    build_args: Vec<String>,
    needs_network: bool,

    // Dependencies (for comments)
    dependencies: Vec<DependencyContext>,
}

#[derive(Serialize)]
struct DependencyContext {
    original: String,
    sps2_name: String,
}

/// Render recipe template with metadata and build info
pub fn render(
    metadata: &RecipeMetadata,
    build_info: &BuildInfo,
    source: &SourceLocation,
    source_hash: Option<String>,
) -> Result<String> {
    // Create Tera instance with embedded template
    let mut tera = Tera::default();

    // Load the template from the embedded string
    let template_content = include_str!("../templates/recipe.star.tera");
    tera.add_raw_template("recipe.star", template_content)
        .map_err(|e| BuildError::DraftTemplateFailed {
            message: format!("Failed to load template: {e}"),
        })?;

    // Source hash is already determined by the caller

    // Determine if this is a git source
    let is_git_source = matches!(source, SourceLocation::Git(_));

    // Prepare template context using the struct
    let ctx_data = TemplateContext {
        // Basic metadata
        name: metadata.name.clone(),
        version: metadata.version.clone(),
        description: metadata.description.clone(),
        license: metadata.license.clone(),
        homepage: metadata.homepage.clone(),

        // Source information
        source_url: extract_source_url(source),
        source_hash,
        is_git_source,
        git_ref: if is_git_source {
            Some("HEAD".to_string())
        } else {
            None
        },

        // Build system information
        build_system: build_info.build_system.clone(),
        build_function: build_info.build_function.clone(),
        build_args: build_info.build_args.clone(),
        needs_network: build_info.needs_network,

        // Dependencies (converted for template)
        dependencies: build_info
            .dependencies
            .iter()
            .map(|dep| DependencyContext {
                original: dep.original.clone(),
                sps2_name: dep.sps2_name.clone(),
            })
            .collect(),
    };

    // Create Tera context from our struct
    let context =
        Context::from_serialize(&ctx_data).map_err(|e| BuildError::DraftTemplateFailed {
            message: format!("Failed to serialize template context: {e}"),
        })?;

    // Render the template
    tera.render("recipe.star", &context).map_err(|e| {
        BuildError::DraftTemplateFailed {
            message: format!("Failed to render template: {e}"),
        }
        .into()
    })
}

/// Extract source URL from `SourceLocation`
fn extract_source_url(source: &SourceLocation) -> Option<String> {
    match source {
        SourceLocation::Url(url) => Some(url.clone()),
        SourceLocation::Git(git_url) => Some(git_url.clone()),
        SourceLocation::Local(_) | SourceLocation::Archive(_) => {
            // For local sources, we don't have a URL to fetch from
            None
        }
    }
}
