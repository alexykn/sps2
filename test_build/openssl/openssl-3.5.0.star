def metadata():
    """Package metadata"""
    return {
        "name": "openssl",
        "version": "3.5.0",
        "description": """TODO: Add package description""",
        "license": "TODO: Specify license"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Download source
    fetch(ctx, "https://github.com/openssl/openssl/releases/download/openssl-3.5.0/openssl-3.5.0.tar.gz", "c30a6b10be0dc613779f111398559cbbbd34dd67b7cbff101eadc2d4431dda82")

    # Build using autotools build system
    autotools(ctx)
