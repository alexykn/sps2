def metadata():
    """Package metadata for libidn2."""
    return {
        "name": "libidn2",
        "version": "2.3.7",
        "description": "International domain name library (IDNA2008/TR46)",
        "license": "GPL-3.0-or-later",
        "homepage": "https://www.gnu.org/software/libidn/#libidn2",
        "depends": [],
        "build_depends": [],
    }

def build(ctx):
    """Build libidn2 for macOS."""
    # Clean up any leftover files from previous builds
    cleanup(ctx)

    # Fetch the source from GNU FTP
    fetch(ctx, "https://ftp.gnu.org/gnu/libidn/libidn2-2.3.7.tar.gz")

    # Build using autotools
    autotools(ctx, [
        "--disable-dependency-tracking",
        "--disable-silent-rules",
        "--disable-static",
    ])