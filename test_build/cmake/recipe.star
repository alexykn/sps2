def metadata():
    """Package metadata"""
    return {
        "name": "hello-cmake",
        "version": "1.0.0",
        "description": "Test CMake build system",
        "license": "MIT"
    }

def build(ctx):
    # Use the cmake helper function
    cmake(ctx, ["-DCMAKE_INSTALL_PREFIX=" + ctx.PREFIX])