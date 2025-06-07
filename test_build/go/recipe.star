def metadata():
    """Package metadata"""
    return {
        "name": "hello-go",
        "version": "1.0.0",
        "description": "Test Go build system",
        "license": "MIT"
    }

def build(ctx):
    # Use the go helper function
    go(ctx, ["build"])