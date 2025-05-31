# Build recipe for hello - a simple test program
def metadata():
    return {
        "name": "hello",
        "version": "1.0.0",
        "description": "A simple hello world program",
        "homepage": "https://github.com/sps2/hello",
        "license": "MIT"
    }

def build(ctx):
    # Build the hello program using the provided context
    # For this simple test, we'll use the local source files

    # Access context attributes
    # ctx.NAME - package name
    # ctx.VERSION - package version
    # ctx.PREFIX - installation prefix
    # ctx.JOBS - number of parallel jobs

    # TODO: Methods (fetch, make, install) are not yet callable from Starlark
    # The build system will need to handle the actual compilation for now
    pass
