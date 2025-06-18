def metadata():
    """Package metadata for libssh2."""
    return {
        "name": "libssh2",
        "version": "1.11.1",
        "description": "A client-side C library implementing the SSH2 protocol",
        "license": "BSD",
        "homepage": "https://libssh2.org",
        "depends": [
            "openssl",
            "zlib",
        ],
        "build_depends": [],
    }

def build(ctx):
    """Build libssh2 for macOS."""
    # Clean up any leftover files from previous builds
    cleanup(ctx)

    # Fetch the source from GitHub
    fetch(ctx, "https://github.com/libssh2/libssh2/releases/download/libssh2-1.11.1/libssh2-1.11.1.tar.gz")

    # Build using CMake
    cmake(ctx, [
        "-DCMAKE_BUILD_TYPE=Release",
        "-DBUILD_SHARED_LIBS=ON",
        "-DENABLE_ZLIB_COMPRESSION=ON",
        "-DBUILD_EXAMPLES=OFF",
        "-DBUILD_TESTING=OFF",
    ])