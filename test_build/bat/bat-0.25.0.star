def metadata():
    """Package metadata"""
    return {
        "name": "bat",
        "version": "0.25.0",
        "description": """A cat(1) clone with wings.""",
        "license": "MIT OR Apache-2.0",
        "homepage": "https://github.com/sharkdp/bat"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Clone git repository
    git(ctx, "https://github.com/sharkdp/bat", "HEAD")
    # Allow network access for dependency downloads
    allow_network(ctx, True)

    # Build using cargo build system
    cargo(ctx, [
        "--release"
    ])
