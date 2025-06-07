def metadata():
    """Package metadata"""
    return {
        "name": "hello-yarn",
        "version": "1.0.0",
        "description": "Test Node.js yarn build system",
        "license": "MIT"
    }

def build(ctx):
    # Just copy the script
    command(ctx, "mkdir -p stage/bin")
    command(ctx, "cp src/hello.js stage/bin/hello-yarn")
    command(ctx, "chmod +x stage/bin/hello-yarn")