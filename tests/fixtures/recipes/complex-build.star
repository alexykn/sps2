# Complex build recipe with dependencies

def metadata():
    """Return package metadata as a dictionary."""
    return {
        "name": "complex-app",
        "version": "2.1.3",
        "description": "A complex application with multiple dependencies",
        "license": "Apache-2.0",
        "homepage": "https://github.com/example/complex-app",
        "depends": [
            "libssl>=1.1.1,<2.0",
            "zlib~=1.2.11",
            "sqlite>=3.36.0",
            "curl>=7.68.0,<8.0"
        ],
        "build_depends": [
            "cmake>=3.16",
            "gcc>=9.0",
            "pkg-config>=0.29"
        ]
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
    # Fetch source with verification
    fetch(ctx, "https://github.com/example/complex-app/archive/v2.1.3.tar.gz")
    
    # Extract source (handled automatically by fetch)
    # Enter source directory
    command(ctx, "cd complex-app-2.1.3")
    
    # Use CMake build system
    cmake(ctx, [
        "-DCMAKE_INSTALL_PREFIX=" + ctx.PREFIX,
        "-DCMAKE_BUILD_TYPE=Release",
        "-DWITH_SSL=ON",
        "-DWITH_SQLITE=ON",
        "-S", "complex-app-2.1.3",
        "-B", "build"
    ])
    
    # Build with make using parallel jobs
    command(ctx, "cd build")
    make(ctx, ["-C", "build", "-j" + str(ctx.JOBS)])
    
    # Run tests
    make(ctx, ["-C", "build", "test"])
    
    # Install to staging directory
    make(ctx, ["-C", "build", "install", "DESTDIR=$(pwd)/stage"])
    
    # Install additional documentation (files go directly in stage/)
    command(ctx, "mkdir -p stage/share/doc/complex-app")
    command(ctx, "cp complex-app-2.1.3/README.md complex-app-2.1.3/CHANGELOG.md complex-app-2.1.3/LICENSE stage/share/doc/complex-app/")
    command(ctx, "cp -r complex-app-2.1.3/docs/ stage/share/doc/complex-app/")
    
    # Create sample configuration
    command(ctx, "mkdir -p stage/share/complex-app")
    command(ctx, """cat > stage/share/complex-app/config.example.toml << 'EOF'
[application]
name = "complex-app"
version = "2.1.3"
debug = false

[database]
type = "sqlite"
path = "~/.local/share/complex-app/data.db"

[network]
timeout = 30
retries = 3
EOF""")