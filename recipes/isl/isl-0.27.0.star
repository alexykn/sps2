#
# sps2 build recipe for the isl (Integer Set Library)
#
# isl is a library for manipulating sets and relations of integer points
# bounded by linear constraints. It is a dependency for GCC's
# Graphite loop optimization framework.
#

def metadata():
    """Return package metadata for isl."""
    return {
        "name": "isl",
        "version": "0.27.0",
        "description": "A library for manipulating sets and relations of integer points bounded by linear constraints.",
        "license": "MIT",
        "homepage": "https://libisl.sourceforge.io/",
        "depends": [
            "gmp",
        ],
        "build_depends": [
            "gmp",
        ],
    }

def build(ctx):
    """Build the package using the provided context."""
    # 1. Start with a clean build environment.
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)

    # 2. Fetch the source code from the official mirror.
    fetch(ctx, "https://libisl.sourceforge.io/isl-0.27.tar.bz2")

    # 3. Configure, build, and stage the package using the autotools helper.
    autotools(ctx, [
        # Build shared libraries for dynamic linking.
        "--enable-shared",
        # Disable the static library to save space and reduce complexity.
        "--disable-static",
    ])
