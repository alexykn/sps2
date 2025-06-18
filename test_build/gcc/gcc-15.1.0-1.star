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
    #fetch(ctx, "https://ftp.gnu.org/gnu/gcc/gcc-15.1.0/gcc-15.1.0.tar.gz")

    # using local source for now
    copy(ctx)

    # Apply the Darwin patch from Homebrew (required for macOS support)
    apply_patch(ctx, "gcc-15.1.0-darwin.patch")
    
    # Set environment variables for the build
    # Add rpath to help find libraries at runtime
    set_env(ctx, "LDFLAGS", "-L" + ctx.PREFIX + "/lib -L/usr/lib -Wl,-rpath," + ctx.PREFIX + "/lib -Wl,-rpath,/usr/lib")
    set_env(ctx, "CPPFLAGS", "-I" + ctx.PREFIX + "/include")
    set_env(ctx, "DYLD_LIBRARY_PATH", ctx.PREFIX + "/lib:/usr/lib")
    
    # Also set BOOT_LDFLAGS for GCC's internal build process
    set_env(ctx, "BOOT_LDFLAGS", "-Wl,-headerpad_max_install_names -Wl,-rpath," + ctx.PREFIX + "/lib -Wl,-rpath,/usr/lib")

    # 3. Create build directory (GCC requires out-of-source builds)
    command(ctx, ["mkdir", "-p", "build"])

    # 4. Configure the build optimized for macOS ARM64
    # Note: GCC on macOS requires specific handling
    
    # Build triple for Darwin (following Homebrew's approach)
    build_triple = "aarch64-apple-darwin24"
    
    # Get SDK path
    sdk_path = "/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk"
    
    # Run configure from build directory with all options
    configure_cmd = "cd build && ../configure " + \
        "--prefix=" + ctx.PREFIX + " " + \
        "--build=" + build_triple + " " + \
        "--with-sysroot=" + sdk_path + " " + \
        "--with-native-system-header-dir=/usr/include " + \
        "--with-gmp=" + ctx.PREFIX + " " + \
        "--with-mpfr=" + ctx.PREFIX + " " + \
        "--with-mpc=" + ctx.PREFIX + " " + \
        "--with-isl=" + ctx.PREFIX + " " + \
        "--with-zstd=" + ctx.PREFIX + " " + \
        "--enable-languages=c,c++,objc,obj-c++,fortran " + \
        "--disable-multilib " + \
        "--enable-checking=release " + \
        "--with-gcc-major-version-only " + \
        "--with-system-zlib " + \
        "--disable-nls " + \
        "--disable-bootstrap"
    
    command(ctx, ["sh", "-c", configure_cmd])
    
    # 5. Build GCC in the build directory
    # Pass BOOT_LDFLAGS to ensure proper library paths during build
    make_cmd = "cd build && make -j" + str(ctx.JOBS) + " BOOT_LDFLAGS='-Wl,-headerpad_max_install_names -Wl,-rpath," + ctx.PREFIX + "/lib -Wl,-rpath,/usr/lib'"
    command(ctx, ["sh", "-c", make_cmd])
    
    # 6. Install GCC from the build directory
    command(ctx, ["sh", "-c", "cd build && make install"])
