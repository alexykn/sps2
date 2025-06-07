def metadata():
    """Package metadata"""
    return {
        "name": "hello-setuppy",
        "version": "1.0.0",
        "description": "Test Python setup.py build system",
        "license": "MIT"
    }

def build(ctx):
    # Create bin directory
    command(ctx, "mkdir -p stage/bin")
    
    # Copy the script directly
    command(ctx, "cp src/hello.py stage/bin/hello-setuppy")
    command(ctx, "chmod +x stage/bin/hello-setuppy")