def metadata():
    """Package metadata"""
    return {
        "name": "hello-pyproject",
        "version": "1.0.0",
        "description": "Test Python pyproject.toml build system",
        "license": "MIT"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)

    # Allow network access for pip to download dependencies
    allow_network(ctx, True)

    # Use the python() function which handles setup.py and pyproject.toml
    # The build system will automatically detect pyproject.toml
    python(ctx)