#
# sps2 build recipe for the GNU MPC Library
#
# MPC is a C library for the arithmetic of complex numbers with
# arbitrarily high precision and correct rounding of the result.
# It is a key dependency for GCC.
#

def metadata():
    """Return package metadata for MPC."""
    return {
        "name": "mpc",
        "version": "1.3.1",
        "description": "A C library for complex number arithmetic with arbitrarily high precision and correct rounding.",
        "license": "LGPL-3.0-or-later",
        "homepage": "https://www.multiprecision.org/mpc/",
        "depends": [
            "gmp",
            "mpfr",
        ],
        "build_depends": [],
    }

def build(ctx):
    """Build the package using the provided context."""
    # 1. Start with a clean build environment.
    cleanup(ctx)

    # 2. Fetch the source code from the official GNU FTP server.
    fetch(ctx, "https://ftp.gnu.org/gnu/mpc/mpc-1.3.1.tar.gz")

    # 3. Configure, build, and stage the package using the autotools helper.
    autotools(ctx, [
        # Link against the GMP and MPFR libraries provided in the build environment.
        "--with-gmp=" + ctx.PREFIX,
        "--with-mpfr=" + ctx.PREFIX,

        # Build shared libraries for dynamic linking.
        "--enable-shared",

        # Disable the static library to save space.
        "--disable-static",
    ])
