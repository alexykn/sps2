def metadata():
    """Package metadata"""
    return {
        "name": "tar",
        "version": "1.35.0",
        "description": "GNU tar archiving utility",
        "license": "GPL-3.0-or-later"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)
    
    # Fetch release tarball
    fetch(ctx, "https://ftp.gnu.org/gnu/tar/tar-1.35.tar.gz")

    # Build using autotools build system
    # No bootstrap needed for release tarballs
    # On macOS, we need to explicitly link with iconv
    autotools(ctx, ["LIBS=-liconv"])
