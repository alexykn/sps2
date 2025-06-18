def metadata():
    """Package metadata for GNU Make."""
    return {
        "name": "make",
        "version": "4.4.1",
        "description": "GNU Make is a tool which controls the generation of executables and other non-source files of a program from the program's source files.",
        "license": "GPL-3.0-or-later",
        "homepage": "https://www.gnu.org/software/make/",
        "build_depends": [],
    }

def build(ctx):
    """Build GNU Make for macOS."""
    # 1. Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)

    # 2. Fetch the source archive from the official GNU FTP server
    fetch(ctx, "https://ftp.gnu.org/gnu/make/make-4.4.1.tar.gz")
    # 3. Force GCC to avoid clang issues
    set_env(ctx, "CC", "gcc")
    set_env(ctx, "CXX", "g++")

    # 4. Build using autotools with macOS-specific configuration
    autotools(ctx, [
        # Install as 'gmake' to avoid conflict with system make
        # "--program-prefix=g", -- I don't care, manage your paths right. :P
        # Standard optimization flags
        "--disable-dependency-tracking",
        "--disable-silent-rules",
        # Skip optional Guile support
        "--without-guile"
    ])
