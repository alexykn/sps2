def metadata():
    """Package metadata"""
    return {
        "name": "hello-cmake",
        "version": "1.0.0",
        "description": "Test CMake build system",
        "license": "MIT"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Use the cmake() function which handles cmake configuration and build
    # Don't pass CMAKE_INSTALL_PREFIX manually, let the build system handle it
    cmake(ctx, ["-DCMAKE_BUILD_TYPE=Release"])