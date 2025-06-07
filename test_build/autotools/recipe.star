def metadata():
    """Package metadata"""
    return {
        "name": "hello-autotools",
        "version": "1.0.0",
        "description": "Test autotools build system",
        "license": "MIT"
    }

def build(ctx):
    # Use the autotools helper function
    # The autotools build system will automatically run autoreconf if needed
    autotools(ctx, ["--prefix=" + ctx.PREFIX])