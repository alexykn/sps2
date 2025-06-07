def metadata():
    """Package metadata"""
    return {
        "name": "test-make",
        "version": "1.0.0",
        "description": "Test package for make build system",
        "license": "MIT"
    }

def build(ctx):
    # Build the program using make
    make(ctx, ["-j" + str(ctx.JOBS)])
    
    # Install files to staging directory
    make(ctx, ["install", "DESTDIR=stage"])