def metadata():
    """Package metadata"""
    return {
        "name": "pkgconf",
        "version": "2.4.3",
        "description": """TODO: Add package description""",
        "license": "TODO: Specify license"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Clone git repository
    git(ctx, "https://github.com/pkgconf/pkgconf", "HEAD")

    # Build using meson build system
    meson(ctx, [
        "--buildtype=release"
    ])
