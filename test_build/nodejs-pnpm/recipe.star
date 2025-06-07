def metadata():
    """Package metadata"""
    return {
        "name": "hello-pnpm",
        "version": "1.0.0",
        "description": "Test Node.js pnpm build system",
        "license": "MIT"
    }

def build(ctx):
    # Just copy the script
    command(ctx, "mkdir -p stage/bin")
    command(ctx, "cp src/hello.js stage/bin/hello-pnpm")
    command(ctx, "chmod +x stage/bin/hello-pnpm")