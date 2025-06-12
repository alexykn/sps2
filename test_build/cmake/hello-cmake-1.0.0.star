def metadata():
    """Package metadata"""
    return {
        "name": "hello-cmake",
        "version": "1.0.0",
        "description": "TODO: Add package description",
        "license": "TODO: Specify license"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Build using cmake build system
    cmake(ctx, [
        "-DCMAKE_BUILD_TYPE=Release"
    ])