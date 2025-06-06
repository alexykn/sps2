# Simple hello world package recipe for testing

def metadata():
    """Return package metadata as a dictionary."""
    return {
        "name": "hello-world",
        "version": "1.0.0",
        "description": "Simple hello world package for testing",
        "license": "MIT",
        "homepage": "https://example.com/hello-world",
        "depends": [],  # No runtime dependencies
        "build_depends": []  # No build dependencies
    }

def build(ctx):
    """Build the package using the provided context.
    
    Args:
        ctx: Build context with attributes:
            - ctx.NAME: package name from metadata
            - ctx.VERSION: package version from metadata
            - ctx.PREFIX: installation prefix (e.g. /opt/pm/live)
            - ctx.JOBS: number of parallel build jobs
    """
    # Fetch source
    fetch(ctx, "https://example.com/hello-world-1.0.0.tar.gz")
    
    # Create source directory
    command(ctx, "mkdir -p src")
    
    # Create hello.c file
    command(ctx, "cat > src/hello.c << 'EOF'\n#include <stdio.h>\n\nint main() {\n    printf(\"Hello, World!\\n\");\n    return 0;\n}\nEOF")
    
    # Compile using make (or could use cmake, autotools, etc)
    # For a simple C file, we use make with custom target
    command(ctx, "echo 'hello: src/hello.c' > Makefile")
    command(ctx, "echo '\t$(CC) -o hello src/hello.c' >> Makefile")
    command(ctx, "echo 'install: hello' >> Makefile")
    command(ctx, "echo '\tmkdir -p $(DESTDIR)$(PREFIX)/bin' >> Makefile")
    command(ctx, "echo '\tcp hello $(DESTDIR)$(PREFIX)/bin/' >> Makefile")
    
    # Build with make
    make(ctx, ["-j" + str(ctx.JOBS)])
    
    # Install to staging directory
    make(ctx, ["install", "DESTDIR=$(pwd)/stage", "PREFIX=" + ctx.PREFIX])
    
    # Add documentation (files go directly in stage/)
    command(ctx, "mkdir -p stage/share/doc/hello-world")
    command(ctx, "echo '# Hello World\n\nA simple hello world program.' > README.md")
    command(ctx, "cp README.md stage/share/doc/hello-world/")