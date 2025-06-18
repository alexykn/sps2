def metadata():
    """Package metadata"""
    return {
        "name": "nghttp2",
        "version": "1.65.0",
        "description": """TODO: Add package description""",
        "license": "TODO: Specify license"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Download source
    fetch(ctx, "https://github.com/nghttp2/nghttp2/releases/download/v1.65.0/nghttp2-1.65.0.tar.bz2")

    # Build using autotools build system
    autotools(ctx)
