//! YAML recipe execution

use crate::yaml::RecipeMetadata;
use crate::{BuildConfig, BuildContext, BuildEnvironment};
use sps2_errors::Error;
use sps2_types::package::PackageSpec;

/// Execute the YAML recipe and return dependencies, metadata, and install request status
pub async fn execute_recipe(
    config: &BuildConfig,
    context: &BuildContext,
    environment: &mut BuildEnvironment,
) -> Result<(Vec<String>, Vec<PackageSpec>, RecipeMetadata, bool), Error> {
    // Execute YAML recipe using staged execution
    crate::utils::executor::execute_staged_build(config, context, environment).await
}
