def metadata():
    """Package metadata"""
    return {
        "name": "hello-cargo",
        "version": "1.0.0",
        "description": "Test Cargo (Rust) build system",
        "license": "MIT"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Use the cargo() function which handles cargo build
    # cargo() already adds "build" command, just pass additional args
    cargo(ctx, ["--release"])
    
    # Cargo's install() will handle the staging correctly