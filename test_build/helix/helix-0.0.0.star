def metadata():
    """Package metadata"""
    return {
        "name": "helix",
        "version": "25.1.1",
        "description": """vim but in rust""",
        "license": "MIT"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Clone git repository
    git(ctx, "https://github.com/helix-editor/helix", "HEAD")
    # Allow network access for dependency downloads
    allow_network(ctx, True)

    # Build using cargo build system
    cargo(ctx, [
        "--release"
    ])
