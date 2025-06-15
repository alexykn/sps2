#
# sps2 build recipe for GNU Binutils
#
# This recipe builds binutils, a collection of essential binary tools
# including the linker (ld) and the assembler (as).
#

def metadata():
    """Return package metadata for GNU Binutils."""
    return {
        "name": "binutils",
        "version": "2.44.0",
        "description": "The GNU Binutils are a collection of binary tools, including the linker, assembler, and other tools for object file manipulation.",
        "license": "GPL-3.0-or-later",
        "homepage": "https://www.gnu.org/software/binutils/",
        "build_depends": [
            "zlib",  # For handling compressed debug sections
        ],
    }

def build(ctx):
    """Build the package using the provided context."""
    # 1. Start with a clean build environment.
    cleanup(ctx)

    # 2. Fetch the source code from the GNU FTP server.
    fetch(ctx, "https://ftp.gnu.org/gnu/binutils/binutils-2.44.tar.gz", "c7808a2027fb40e616c10b4673dac0ad57f043be4166e7700df780c613005b55")

    # 3. Configure the build using the autotools helper with standard flags.
    autotools(ctx, [
        # Use the system's zlib library.
        "--with-system-zlib",

        # Build shared libraries, which are needed by other tools.
        "--enable-shared",

        # Disable building for multiple architectures to keep the package focused.
        "--disable-multilib",

        # Disable Native Language Support to reduce package size.
        "--disable-nls",

        # Disable CTF support (not available on macOS)
        "--disable-libctf",
    ])

    # 4. Compile the package.
    make(ctx)
