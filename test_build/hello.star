def metadata():
    return {
        "name": "hello-world",
        "version": "1.0.0",
        "description": "Simple hello world program",
        "license": "MIT",
        "homepage": "https://example.com/hello-world",
            }
def build(ctx):
    # Build the hello world program using make
    ctx.make()
    
    # Install to staging directory
    ctx.install()
