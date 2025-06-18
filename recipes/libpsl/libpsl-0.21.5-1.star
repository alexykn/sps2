def metadata():
    """Package metadata for libpsl."""
    return {
        "name": "libpsl",
        "version": "0.21.5",
        "description": "C library for the Public Suffix List",
        "license": "MIT",
        "homepage": "https://github.com/rockdaboot/libpsl",
        "depends": [
            "libidn2",
        ],
        "build_depends": [],
    }

def build(ctx):
    """Build libpsl for macOS."""
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)

    # Fetch the source from GitHub
    fetch(ctx, "https://github.com/rockdaboot/libpsl/releases/download/0.21.5/libpsl-0.21.5.tar.gz")

    # Build using autotools
    autotools(ctx, [
        "--disable-dependency-tracking",
        "--disable-silent-rules",
        "--disable-static",
        "--enable-runtime=libidn2",
        "--enable-builtin=libidn2",
    ])