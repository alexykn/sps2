def metadata():
    """Package metadata"""
    return {
        "name": "openssl",
        "version": "3.5.0",
        "description": "Robust, commercial-grade, and full-featured toolkit for TLS and SSL protocols",
        "license": "Apache-2.0"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    
    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)
    
    # Download source
    fetch(ctx, "https://github.com/openssl/openssl/releases/download/openssl-3.5.0/openssl-3.5.0.tar.gz")

    # Configure using OpenSSL's Configure script
    configure(ctx, [
        "darwin64-arm64-cc",
        "--prefix=/opt/pm/live",
        "--openssldir=/opt/pm/live/etc/ssl",
        "--libdir=lib",
        "shared",
        "zlib-dynamic"
    ])
    
    # Build and install
    make(ctx, [])
    make(ctx, ["install"])
