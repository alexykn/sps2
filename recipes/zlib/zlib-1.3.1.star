def metadata():
    """Package metadata"""
    return {
        "name": "zlib",
        "version": "1.3.1",
        "description": """A massively spiffy yet delicately unobtrusive compression library.""",
        "license": "Zlib"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)
    
    # Download source
    fetch(ctx, "https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz")

    # Build using cmake build system
    cmake(ctx, [
        "-DCMAKE_BUILD_TYPE=Release"
    ])
