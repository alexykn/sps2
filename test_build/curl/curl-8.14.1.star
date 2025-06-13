def metadata():
    """Package metadata"""
    return {
        "name": "curl",
        "version": "8.14.1",
        "description": """curl""",
        "license": "TODO: Specify license"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Clone git repository
    git(ctx, "https://github.com/curl/curl", "HEAD")

    # Build using cmake build system
    cmake(ctx, [
        "-DCMAKE_BUILD_TYPE=Release"
    ])
