#
# sps2 build recipe for the GNU MPFR Library
#
# MPFR is a C library for multiple-precision floating-point computations
# with correct rounding. It is a dependency for GCC.
#

def metadata():
    """Return package metadata for MPFR."""
    return {
        "name": "mpfr",
        "version": "4.2.2",
        "description": "A C library for multiple-precision floating-point computations with correct rounding.",
        "license": "LGPL-3.0-or-later",
        "homepage": "https://www.mpfr.org/",
        "depends": [
            "gmp",
        ],
        "build_depends": [],
    }

def build(ctx):
    """Build the package using the provided context."""
    # 1. Start with a clean build environment.
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)

    # 2. Fetch the source code from the official MPFR website.
    fetch(ctx, "https://www.mpfr.org/mpfr-current/mpfr-4.2.2.tar.gz")

    # 3. Configure, build, and stage the package using the autotools helper.
    autotools(ctx, [
        # Link against the GMP library provided in the build environment.
        "--with-gmp=" + ctx.PREFIX,

        # Build shared libraries for dynamic linking.
        "--enable-shared",

        # Disable the static library to save space.
        "--disable-static",
    ])
