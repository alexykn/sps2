use sps2_builder::{BuildConfig, BuildContext, Builder};
use sps2_types::Version;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing sps2 builder with hello package...");

    // Create build context
    let context = BuildContext::new(
        "hello".to_string(),
        Version::parse("1.0.0")?,
        PathBuf::from("hello.star"),
        PathBuf::from("."),
    );

    // Create builder with network enabled (for file:// URLs)
    let config = BuildConfig::with_network();
    let builder = Builder::with_config(config);

    // Build the package
    match builder.build(context).await {
        Ok(result) => {
            println!("Build successful!");
            println!("Package created at: {}", result.package_path.display());
        }
        Err(e) => {
            eprintln!("Build failed: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}
