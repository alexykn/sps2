#
# sps2 build recipe for the GNU Compiler Collection (GCC)
#
# This recipe builds GCC 15.1.0, including support for C and C++ languages.
# It correctly configures the build with its required libraries:
# gmp, mpfr, and mpc.
#

def metadata():
    """Return package metadata for GCC."""
    return {
        "name": "gcc",
        "version": "15.1.0",
        "description": "The GNU Compiler Collection (GCC) is a comprehensive suite of compilers for various programming languages.",
        "license": "GPL-3.0-or-later",
        "homepage": "https://gcc.gnu.org",
        "build_depends": [
            "binutils", # Provides the assembler and linker
            "gmp",      # GNU Multiple Precision Arithmetic Library
            "mpfr",     # GNU Multiple-Precision Floating-Point Library
            "mpc",      # GNU Multiple-Precision Complex Library
            "isl",      # Integer Set Library for loop optimizations
            "zlib",     # Compression library
        ],
    }

def build(ctx):
    """Build the package using the provided context."""
    # 1. Clean up any leftover files from previous builds.
    cleanup(ctx)

    # 2. Fetch the source archive from the official GNU FTP server.
    fetch(ctx, "https://ftp.gnu.org/gnu/gcc/gcc-15.1.0/gcc-15.1.0.tar.gz", "237f49dc296fce30af526426c06906bb1e774b0ec08b75aa4caef04442167f90")

    # 3. Configure the build using the autotools helper.
    # We pass several flags to ensure GCC is built correctly with all its
    # dependencies and required languages.
    autotools(ctx, [
        # Point the build to the required libraries.
        # The sps2 builder makes these available in the build environment.
        "--with-gmp=" + ctx.PREFIX,
        "--with-mpfr=" + ctx.PREFIX,
        "--with-mpc=" + ctx.PREFIX,
        "--with-isl=" + ctx.PREFIX,

        # Specify the languages to build (C and C++).
        "--enable-languages=c,c++",

        # Disable building for multiple architectures to simplify the process
        # and reduce build time, as we are targeting a single architecture.
        "--disable-multilib",
    ])
