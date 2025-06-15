def metadata():
    """Package metadata for GNU Make."""
    return {
        "name": "make",
        "version": "4.4",
        "description": "GNU Make is a tool which controls the generation of executables and other non-source files of a program from the program's source files.",
        "license": "GPL-3.0-or-later",
        "homepage": "https://www.gnu.org/software/make/",
        "build_depends": [
            "gcc",
        ],
    }

def build(ctx):
    """Build the package using the provided context."""
    # 1. Clean up any leftover files from previous builds to ensure a clean slate.
    cleanup(ctx)

    # 2. Fetch the source archive from the official GNU FTP server.
    # The BLAKE3 hash is provided for integrity verification.
    fetch(ctx, "https://ftp.gnu.org/gnu/make/make-4.4.tar.gz", "1a0e5353205e106bd9b3c0f4a5f37ee1156a1e1c8feb771d1b4842c216612cba")

    # 3. Build using the autotools build system with standard configuration flags.
    # The sps2 builder automatically handles the installation prefix.
    autotools(ctx, [
        # Disables verbose, unnecessary dependency tracking to speed up the build.
        "--disable-dependency-tracking",
        # Ensures build rules are not silenced, providing clearer output for debugging.
        "--disable-silent-rules",
        # Disables the optional Guile integration, which is not needed for a standard build.
        "--without-guile"
    ])
