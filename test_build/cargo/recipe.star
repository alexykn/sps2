def metadata():
    """Package metadata"""
    return {
        "name": "hello-cargo",
        "version": "1.0.0",
        "description": "Test Cargo build system",
        "license": "MIT"
    }

def build(ctx):
    # Build with cargo (it automatically adds "build --release")
    # The source is already in src/ directory
    command(ctx, "cd src && cargo build --release")
    
    # Install binary to stage
    command(ctx, "mkdir -p stage/bin")
    command(ctx, "cp src/target/release/hello-cargo stage/bin/")