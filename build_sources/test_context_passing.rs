use spsv2_package::{load_recipe, RecipeEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load the test recipe
    let recipe = load_recipe("test_context.star").await?;
    
    // Execute it
    let engine = RecipeEngine::new();
    let result = engine.execute(&recipe)?;
    
    println!("Recipe executed successfully!");
    println!("Metadata: {:?}", result.metadata);
    println!("Build steps: {:?}", result.build_steps);
    
    Ok(())
}