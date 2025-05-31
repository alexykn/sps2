use sps2_package::{execute_recipe, load_recipe};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing hello.star recipe...");

    // Load the recipe
    let recipe_path = PathBuf::from("hello.star");
    let recipe = load_recipe(&recipe_path).await?;

    // Execute the recipe to get metadata and build steps
    let result = execute_recipe(&recipe)?;

    // Print metadata
    println!("\nMetadata:");
    println!("  Name: {}", result.metadata.name);
    println!("  Version: {}", result.metadata.version);
    println!("  Description: {:?}", result.metadata.description);
    println!("  Homepage: {:?}", result.metadata.homepage);
    println!("  License: {:?}", result.metadata.license);
    println!("  Runtime deps: {:?}", result.metadata.runtime_deps);
    println!("  Build deps: {:?}", result.metadata.build_deps);

    // Print build steps
    println!("\nBuild steps:");
    for (i, step) in result.build_steps.iter().enumerate() {
        println!("  {}: {:?}", i + 1, step);
    }

    Ok(())
}
