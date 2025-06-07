def metadata():
    """Package metadata"""
    return {
        "name": "hello-autotools",
        "version": "1.0.0",
        "description": "Test autotools build system",
        "license": "MIT"
    }

def build(ctx):
    # Use the autotools function which handles everything:
    # - autoreconf -fi (if configure.ac exists but configure doesn't)
    # - ./configure with args
    # - make
    # - make install DESTDIR=stage
    autotools(ctx, ["--prefix=" + ctx.PREFIX])