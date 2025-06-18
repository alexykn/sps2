def metadata():
    """Package metadata"""
    return {
        "name": "tar",
        "version": "1.35.0",
        "description": """TODO: Add package description""",
        "license": "TODO: Specify license"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Fetch release tarball
    fetch(ctx, "https://ftp.gnu.org/gnu/tar/tar-1.35.tar.gz")

    # Build using autotools build system
    # No bootstrap needed for release tarballs
    # On macOS, we need to explicitly link with iconv
    autotools(ctx, ["LIBS=-liconv"])
