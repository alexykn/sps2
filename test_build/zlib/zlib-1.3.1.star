def metadata():
    """Package metadata"""
    return {
        "name": "zlib",
        "version": "1.3.1",
        "description": """A massively spiffy yet delicately unobtrusive compression library.""",
        "license": "TODO: Specify license"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Download source
    fetch(ctx, "https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz", "207c3b0862cb4e3686f8405f76a98c38dbad9c94bcf4be4b9efca0716aba51ec")

    # Build using cmake build system
    cmake(ctx, [
        "-DCMAKE_BUILD_TYPE=Release"
    ])
