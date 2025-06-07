def metadata():
    """Package metadata"""
    return {
        "name": "hello-npm",
        "version": "1.0.0",
        "description": "Test Node.js npm build system",
        "license": "MIT"
    }

def build(ctx):
    # Just copy the script
    command(ctx, "mkdir -p stage/bin")
    command(ctx, "cp src/hello.js stage/bin/hello-npm")
    command(ctx, "chmod +x stage/bin/hello-npm")