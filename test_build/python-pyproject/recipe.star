def metadata():
    """Package metadata"""
    return {
        "name": "hello-pyproject",
        "version": "1.0.0",
        "description": "Test Python pyproject.toml build system",
        "license": "MIT"
    }

def build(ctx):
    # For now, just copy the script
    command(ctx, "mkdir -p stage/bin")
    command(ctx, "cp src/hello.py stage/bin/hello-pyproject")
    command(ctx, "chmod +x stage/bin/hello-pyproject")