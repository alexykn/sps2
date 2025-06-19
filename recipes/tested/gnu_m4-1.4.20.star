#
# sps2 build recipe for GNU M4
#
# M4 is a macro processor, a standard tool in most Unix-like systems,
# and a build dependency for many GNU packages.
#

def metadata():
    """Return package metadata for GNU M4."""
    return {
        "name": "m4",
        "version": "1.4.20",
        "description": "GNU M4 is an implementation of the traditional Unix macro processor.",
        "license": "GPL-3.0-or-later",
        "homepage": "https://www.gnu.org/software/m4/",
        "build_depends": [],
    }

def build(ctx):
    """Build the package using the provided context."""
    # 1. Start with a clean build environment.
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)

    # 2. Fetch the source code from the official GNU FTP server.
    fetch(ctx, "https://ftp.gnu.org/gnu/m4/m4-1.4.20.tar.gz")

    # 3. Configure, build, and stage the package. M4 has a standard
    # autotools build process with no special flags required.
    autotools(ctx)
