def metadata():
    """Package metadata"""
    return {
        "name": "pkgconf",
        "version": "2.4.3",
        "description": "A system for managing library compile/link flags",
        "license": "ISC"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)
    
    # Fetch stable release archive
    fetch(ctx, "https://distfiles.ariadne.space/pkgconf/pkgconf-2.4.3.tar.xz")

    # Build using meson build system
    meson(ctx, [
        "--buildtype=release"
    ])
