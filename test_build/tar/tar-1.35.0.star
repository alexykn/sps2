def metadata():
    """Package metadata"""
    return {
        "name": "tar",
        "version": "1.35.0",
        "description": """TODO: Add package description""",
        "license": "TODO: Specify license"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Clone git repository
    git(ctx, "https://git.savannah.gnu.org/git/tar.git", "HEAD")

    # Bootstrap the project (required for git version)
    command(ctx, "./bootstrap")
    
    # Build using autotools build system
    autotools(ctx)
