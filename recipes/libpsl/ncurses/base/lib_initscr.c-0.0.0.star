def metadata():
    """Package metadata"""
    return {
        "name": "ncurses/base/lib_initscr.c",
        "version": "0.0.0",
        "description": """TODO: Add package description""",
        "license": "TODO: Specify license"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)
    
    # Download source
    # TODO: Replace with actual BLAKE3 hash after verification
    fetch(ctx, "https://ftp.gnu.org/gnu/ncurses/ncurses-6.5.tar.gz")
    extract(ctx)

    # Build using autotools build system
    autotools(ctx)
