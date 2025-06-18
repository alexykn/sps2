def metadata():
    """Package metadata"""
    return {
        "name": "zstd",
        "version": "1.5.7",
        "description": """Zstandard - Fast lossless compression algorithm targeting real-time compression scenarios at zlib-level and better compression ratios""",
        "license": "BSD-3-Clause OR GPL-2.0",
        "homepage": "https://facebook.github.io/zstd/"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)
    
    # Download source
    fetch(ctx, "https://github.com/facebook/zstd/releases/download/v1.5.7/zstd-1.5.7.tar.gz")

    # Build using make (preferred method according to zstd docs)
    # The make() function automatically handles parallel jobs and prefix configuration
    # We need to set compiler flags for ARM64 optimization
    set_toolchain(ctx, "CC", "clang -arch arm64 -O3")
    set_toolchain(ctx, "CXX", "clang++ -arch arm64 -O3")
    
    # Build and install
    make(ctx)
    make(ctx, ["install"])
