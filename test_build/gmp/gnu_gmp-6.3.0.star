#
# sps2 build recipe for the GNU Multiple Precision Arithmetic Library (GMP)
#
# This library is a crucial dependency for building GCC and other software
# that requires high-precision arithmetic.
#

def metadata():
    """Return package metadata for GMP."""
    return {
        "name": "gmp",
        "version": "6.3.0",
        "description": "A free library for arbitrary precision arithmetic, operating on signed integers, rational numbers, and floating-point numbers.",
        "license": "LGPL-3.0-or-later",
        "homepage": "https://gmplib.org",
        "build_depends": [
            "m4",  # Required by the configure script
        ],
    }

def build(ctx):
    """Build the package using the provided context."""
    # 1. Start with a clean build environment.
    cleanup(ctx)

    # 2. Fetch the source code from the official GMP website.
    fetch(ctx, "https://gmplib.org/download/gmp/gmp-6.3.0.tar.xz", "fffe4996713928ae19331c8ef39129e46d3bf5b7182820656fd4639435cd83a4")

    # 3. Configure, build, and stage the package using the autotools helper.
    # The `autotools` function runs the full `./configure && make && make install` workflow.
    autotools(ctx, [
        # Enable the C++ interface (gmpxx), which is required by other
        # libraries like MPFR and MPC that depend on GMP.
        "--enable-cxx",

        # Build shared libraries for dynamic linking.
        "--enable-shared",

        # Disable the static library to save space and avoid linking issues.
        "--disable-static",
    ])
