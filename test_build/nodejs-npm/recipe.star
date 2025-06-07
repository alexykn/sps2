def metadata():
    """Package metadata"""
    return {
        "name": "hello-npm",
        "version": "1.0.0",
        "description": "Test Node.js npm build system",
        "license": "MIT"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)

    # Use the nodejs() function which handles npm/yarn/pnpm
    # For simple scripts without dependencies, it will just copy the files
    # For projects with dependencies, it will run npm install
    nodejs(ctx)