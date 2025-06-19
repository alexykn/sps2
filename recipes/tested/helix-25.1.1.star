def metadata():
    """Package metadata"""
    return {
        "name": "helix",
        "version": "25.01",
        "description": "A post-modern modal text editor.",
        "license": "MIT"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)
    
    # Fetch stable release archive
    fetch(ctx, "https://github.com/helix-editor/helix/archive/refs/tags/25.01.tar.gz")
    # Allow network access for dependency downloads
    allow_network(ctx, True)

    # Build using cargo build system
    cargo(ctx, [
        "--release"
    ])
