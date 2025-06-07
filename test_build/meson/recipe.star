def metadata():
    """Package metadata"""
    return {
        "name": "hello-meson",
        "version": "1.0.0",
        "description": "Test Meson build system",
        "license": "MIT"
    }

def build(ctx):
    # Use the meson helper function
    meson(ctx, ["--prefix=" + ctx.PREFIX])