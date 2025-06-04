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
    ctx.fetch("https://example.com/hello-world-1.0.0.tar.gz")
    
    # Create source directory
    ctx.command("mkdir -p src")
    
    # Create hello.c file
    ctx.command("cat > src/hello.c << 'EOF'\n#include <stdio.h>\n\nint main() {\n    printf(\"Hello, World!\\n\");\n    return 0;\n}\nEOF")
    
    # Compile using make (or could use ctx.cmake, ctx.autotools, etc)
    # For a simple C file, we use make with custom target
    ctx.command("echo 'hello: src/hello.c' > Makefile")
    ctx.command("echo '\t$(CC) -o hello src/hello.c' >> Makefile")
    ctx.command("echo 'install: hello' >> Makefile")
    ctx.command("echo '\tmkdir -p $(DESTDIR)$(PREFIX)/bin' >> Makefile")
    ctx.command("echo '\tcp hello $(DESTDIR)$(PREFIX)/bin/' >> Makefile")
    
    # Build with make
    ctx.make(["-j" + str(ctx.JOBS)])
    
    # Install to staging directory
    ctx.make(["install", "DESTDIR=$(pwd)/stage", "PREFIX=" + ctx.PREFIX])
    
    # Add documentation
    ctx.command("mkdir -p stage" + ctx.PREFIX + "/share/doc/hello-world")
    ctx.command("echo '# Hello World\n\nA simple hello world program.' > README.md")
    ctx.command("cp README.md stage" + ctx.PREFIX + "/share/doc/hello-world/")