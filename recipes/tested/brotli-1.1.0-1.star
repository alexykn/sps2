def metadata():
    """Package metadata for Brotli compression library."""
    return {
        "name": "brotli",
        "version": "1.1.0",
        "description": "Generic-purpose lossless compression algorithm",
        "license": "MIT",
        "homepage": "https://github.com/google/brotli",
        "depends": [],
        "build_depends": [],
    }

def build(ctx):
    """Build Brotli for macOS."""
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)

    # Fetch the source from GitHub
    fetch(ctx, "https://github.com/google/brotli/archive/refs/tags/v1.1.0.tar.gz")

    # Build using CMake
    cmake(ctx, [
        "-DCMAKE_BUILD_TYPE=Release",
        "-DBUILD_SHARED_LIBS=ON",
        "-DBROTLI_DISABLE_TESTS=ON",
    ])