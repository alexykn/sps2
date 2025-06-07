def metadata():
    """Package metadata"""
    return {
        "name": "hello-meson",
        "version": "1.0.0",
        "description": "Test Meson build system",
        "license": "MIT"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Use the meson() function which handles meson setup, compile, and install
    # Don't pass --prefix manually, let the build system handle it
    meson(ctx, ["--buildtype=release"])