#
# sps2 build recipe for the GNU Compiler Collection (GCC) 15.1.0
#
# Optimized build configuration for macOS ARM64 systems.
# This recipe builds GCC with C and C++ support, properly configured
# for Apple Silicon Macs with native toolchain integration.
#

def metadata():
    """Return package metadata for GCC."""
    return {
        "name": "gcc",
        "version": "15.1.0",
        "description": "The GNU Compiler Collection (GCC) - comprehensive suite of compilers for C, C++, and other languages, optimized for macOS ARM64.",
        "license": "GPL-3.0-or-later",
        "homepage": "https://gcc.gnu.org",
        "build_depends": [
            # Core mathematical libraries required by GCC
            "gmp",      # GNU Multiple Precision Arithmetic Library (version 4.3.2+)
            "mpfr",     # GNU Multiple-Precision Floating-Point Library (version 3.1.0+)
            "mpc",      # GNU Multiple-Precision Complex Library (version 1.0.1+)
            "isl",      # Integer Set Library for Graphite loop optimizations (version 0.15+)
            "zstd",     # For LTO bytecode compression
            # "gettext",  # For internationalization support
            # Note: binutils NOT included - macOS uses native assembler/linker
        ],
    }

def build(ctx):
    """Build GCC optimized for macOS ARM64."""
    # 1. Clean up any leftover files from previous builds
    cleanup(ctx)

    # 2. Fetch the source archive from the official GNU FTP server
    #fetch(ctx, "https://ftp.gnu.org/gnu/gcc/gcc-15.1.0/gcc-15.1.0.tar.gz",
    #      "237f49dc296fce30af526426c06906bb1e774b0ec08b75aa4caef04442167f90")

    # using local source for now
    copy(ctx)

    # 3. Configure the build optimized for macOS ARM64
    autotools(ctx, [
        # === Core Dependencies ===
        "--with-gmp=" + ctx.PREFIX,
        "--with-mpfr=" + ctx.PREFIX,
        "--with-mpc=" + ctx.PREFIX,
        "--with-isl=" + ctx.PREFIX,
        "--with-zstd=" + ctx.PREFIX,

        # === Language Support ===
        "--enable-languages=c,c++",  # Start with C/C++, add others as needed

        # === ARM64 Architecture Optimization ===
        "--target=aarch64-apple-darwin",  # Explicit target for ARM64 macOS
        "--with-arch=armv8-a",           # ARMv8-A architecture
        "--with-tune=generic",           # Generic tuning for broad compatibility
        "--with-cpu=generic",            # Generic CPU setting

        # === Build Configuration ===
        "--enable-shared",               # Build shared libraries
        "--disable-multilib",            # Single architecture (ARM64 only)
        "--enable-threads=posix",        # POSIX threads support

        # === Optimization and Performance ===
        "--enable-lto",                  # Link-time optimization support
        "--enable-plugin",               # Plugin support for LTO
        "--enable-checking=release",     # Optimized checking for release builds

        # === macOS Integration ===
        "--with-system-zlib",            # Use macOS system zlib
        "--enable-host-shared",          # Build host code with -fPIC for better shared lib support
        "--disable-nls",                 # Disable Native Language Support for simpler build

        # === Security Features ===
        "--enable-default-pie",          # Position Independent Executables by default
        "--enable-default-ssp",          # Stack Smashing Protection by default

        # === Native Toolchain ===
        # Let configure auto-detect macOS native assembler and linker
        # No --with-gnu-as or --with-gnu-ld flags (use native tools)

        # === Build Speed vs Quality Trade-off ===
        "--enable-bootstrap",

        # === Additional Optimizations ===
        "--enable-__cxa_atexit",        # Use __cxa_atexit for C++ destructors
        "--enable-clocale=generic",      # Generic C locale support
        "--enable-gnu-unique-object",    # Enable gnu_unique_object relocation
        "--enable-linker-build-id",      # Enable build-id in binaries
        "--enable-plugin",               # Enable plugin support
        "--enable-gold=default",         # Use gold linker when available

        # === Disable Unnecessary Features ===
        "--disable-libstdcxx-debug",     # Skip debug version of libstdc++
        "--disable-libgomp",             # Skip OpenMP runtime (can enable if needed)
        "--disable-libatomic",           # Skip atomic library (can enable if needed)
        "--disable-libitm",              # Skip transactional memory library
        "--disable-libsanitizer",        # Skip sanitizer runtimes initially

        # === Set reasonable defaults ===
        "--with-diagnostics-color=auto", # Colored diagnostics
        "--enable-objc-gc=auto",         # Objective-C garbage collection (auto-detect)
    ])

    # 4. Optional: Install the package after building
    # Uncomment the next line if you want automatic installation
    # install(ctx)
