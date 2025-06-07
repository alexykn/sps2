def metadata():
    """Package metadata"""
    return {
        "name": "hello-go",
        "version": "1.0.0",
        "description": "Test Go build system",
        "license": "MIT"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)

    # Use the go() function which handles go build
    # The build system will automatically set the output path
    go(ctx)