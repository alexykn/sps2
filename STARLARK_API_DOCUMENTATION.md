# sps2 Starlark Build Script API Documentation

## Overview

sps2 uses Starlark (a Python-like language) for build scripts. Each package recipe is a `.star` file that defines metadata and build instructions. The builder automatically handles packaging - you just need to specify how to build the software.

**Tip**: Use `sps2 draft` to automatically generate recipes from Git repositories or source archives:
```bash
# Generate recipe from Git repository
sps2 draft -g "https://github.com/BurntSushi/ripgrep"

# Generate recipe from source archive
sps2 draft -u "https://example.com/package-1.0.tar.gz"

# Generate recipe from local directory
sps2 draft -p ./my-project

# Generate recipe from local archive
sps2 draft -a ./my-archive.tar.gz
```

## Basic Structure

Every Starlark build script must define two functions:

```starlark
def metadata():
    """Return package metadata as a dictionary."""
    return {
        "name": "package-name",
        "version": "1.0.0",
        "description": "Package description",
        "license": "MIT",
        "homepage": "https://example.com",
        "depends": [],        # Runtime dependencies
        "build_depends": []   # Build-time dependencies
    }

def build(ctx):
    """Build the package using the provided context."""
    # Build instructions here
    pass
```

## Context Attributes (ctx.*)

The `ctx` parameter in the `build()` function provides these read-only attributes:

- `ctx.NAME` - Package name from metadata
- `ctx.VERSION` - Package version from metadata
- `ctx.PREFIX` - Final installation prefix (e.g., `/opt/pm/live`)
- `ctx.JOBS` - Number of parallel build jobs (integer)

## Core Build Functions

### cleanup(ctx)
Clean up the staging directory. This removes all files from the staging directory but keeps the directory itself.

**Best practice**: Always call this at the start of your build to ensure a clean environment.

```starlark
def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)

    # Continue with build...
```

### git(ctx, url, ref)
Clone a git repository. This is the preferred method for fetching source code.

```starlark
# Clone latest commit from default branch
git(ctx, "https://github.com/BurntSushi/ripgrep", "HEAD")

# Clone specific tag
git(ctx, "https://github.com/helix-editor/helix", "v25.1.1")

# Clone specific commit
git(ctx, "https://github.com/user/repo", "abc123def456")
```

### fetch(ctx, url, hash?)
Fetch and extract source archive from URL.

```starlark
# Basic fetch
fetch(ctx, "https://github.com/curl/curl/releases/download/curl-8_14_1/curl-8.14.1.tar.bz2")

# Fetch with explicit hash verification
fetch_blake3(ctx, "https://github.com/curl/curl/releases/download/curl-8_14_1/curl-8.14.1.tar.bz2", "<blake3_hash>")
fetch_md5(ctx, "https://github.com/curl/curl/releases/download/curl-8_14_1/curl-8.14.1.tar.bz2", "<md5_hash>")
fetch_sha256(ctx, "https://github.com/curl/curl/releases/download/curl-8_14_1/curl-8.14.1.tar.bz2", "<sha256_hash>")
```

### allow_network(ctx, enabled)
Enable or disable network access during build. Network is disabled by default for hermetic builds.

```starlark
# Enable network for dependency downloads (Cargo, Go modules, npm, etc.)
allow_network(ctx, True)
```

### with_defaults(ctx)
Apply sps2 recommended compiler flags and environment variables for optimized builds on macOS ARM64.

This function sets:
- C/C++ optimization flags (`-O2`, `-mcpu=apple-m1`, security hardening)
- Rust optimization flags (`RUSTFLAGS` with `target-cpu=apple-m1`)
- Architecture-specific environment variables
- Linker optimization flags

**Best practice**: Call this before your build steps to get optimized, secure builds.

```starlark
def build(ctx):
    cleanup(ctx)

    # Apply optimized compiler flags
    with_defaults(ctx)

    # Continue with build - flags are now optimized
    fetch(ctx, "https://example.com/source.tar.gz")
    autotools(ctx)
```

Without `with_defaults()`, code compiles with minimal optimization (`-O0`). With it, you get:
- Optimization level 2 (`-O2`)
- Apple Silicon CPU targeting
- Security hardening (stack protector, FORTIFY_SOURCE)
- Architecture-optimized code generation

### command(ctx, cmd)
Execute arbitrary shell command. **Use sparingly** - prefer build system functions.

```starlark
# Only use for special cases not covered by build system functions
command(ctx, "special-build-script.sh")
```

### apply_patch(ctx, path)
Apply a patch file to the source.

```starlark
apply_patch(ctx, "fix-macos-build.patch")
```

### install(ctx)
Request installation of the built package after .sp creation.

**Important**:
- Optional - omit if you only want to build, not install
- Must be the LAST step in the recipe if used
- The builder automatically handles packaging

```starlark
def build(ctx):
    cleanup(ctx)
    git(ctx, "https://github.com/example/app", "HEAD")
    cargo(ctx, ["--release"])
    install(ctx)  # Auto-install after build (optional)
```

### patch_rpaths(ctx, paths?)
Convert `@rpath` references in dynamic libraries and executables to absolute paths.

**What it does**:
- Converts all `@rpath/libfoo.dylib` references to `/opt/pm/live/lib/libfoo.dylib`
- Removes build-time RPATH entries that point to build directories
- Ensures binaries work with tools that don't handle `@rpath` correctly

**When to use**:
- When a package fails at runtime with "dylib not found" errors
- When binaries need to work with older tools that don't understand `@rpath`
- For compatibility with certain development tools (e.g., some versions of CMake, pkg-config)

**When NOT to use**:
- Most modern packages work correctly with `@rpath` (the default behavior)
- Packages that need relocatable binaries or frameworks
- When you want the flexibility of RPATH-based library resolution

**Best practices**:
- Try building without `patch_rpaths()` first - most packages work with modern `@rpath`
- Only add `patch_rpaths()` if you encounter runtime linking issues
- Place it after all build steps but before `install()` if used

```starlark
def build(ctx):
    cleanup(ctx)
    fetch(ctx, "https://example.com/source.tar.gz")
    cmake(ctx, ["-DCMAKE_BUILD_TYPE=Release"])

    # Only if needed for compatibility
    patch_rpaths(ctx)

    # Optional auto-install
    install(ctx)
```

**Technical details**:
- By default, sps2 keeps `@rpath` references with proper `LC_RPATH` entries (modern approach)
- `patch_rpaths()` rewrites these to absolute paths for compatibility
- Without arguments, processes all Mach-O files in the staging directory
- With paths argument, only processes specified files/directories

### fix_permissions(ctx, paths?)
Fix executable permissions on binaries that weren't properly set by the build system.

**What it does**:
- Makes all files in `bin/`, `sbin/` directories executable
- Makes dynamic libraries (`.dylib`, `.so`) executable
- Makes scripts with shebang lines (`#!/bin/sh`, etc.) executable
- Makes Mach-O binaries in `libexec/` executable
- Detects and fixes common script extensions (`.sh`, `.py`, `.pl`, etc.)

**When to use**:
- When binaries are installed without execute permissions (common with GCC)
- When shell scripts or other executables lack proper permissions
- After building packages that have inconsistent permission handling

**When NOT to use**:
- Most packages set permissions correctly during installation
- When you need fine-grained permission control

**Best practices**:
- Try building without `fix_permissions()` first
- Only add it if you get "permission denied" errors when running installed programs
- Place it after all build steps but before `install()` if used

```starlark
def build(ctx):
    cleanup(ctx)
    fetch(ctx, "https://example.com/source.tar.gz")
    autotools(ctx)
    
    # Fix permissions on all executables
    fix_permissions(ctx)
    
    # Or fix specific directories only
    fix_permissions(ctx, ["bin/", "libexec/"])
```

**Technical details**:
- The default post-validation only fixes Mach-O binaries and dynamic libraries
- `fix_permissions()` uses aggressive detection for all executable types
- Preserves existing read permissions and converts them to execute permissions
- Without arguments, processes all files in the staging directory
- With paths argument, only processes specified files/directories

## Build System Functions

The builder automatically handles proper installation, prefix configuration, and packaging for each build system. Since sps2 uses a fixed installation prefix (`/opt/pm/live`), you don't need to specify `--prefix` or similar options - the builder configures this for you.

### cargo(ctx, args?)
Build Rust projects with Cargo. The builder automatically handles release builds and binary collection.

```starlark
# Simple release build
cargo(ctx, ["--release"])

# With features
cargo(ctx, ["--release", "--features", "ssl,compression"])

# With tests
cargo(ctx, ["test"])
cargo(ctx, ["--release"])
```

### meson(ctx, args?)
Build projects using Meson. The builder handles the full Meson/Ninja workflow and sets the prefix automatically.

```starlark
# Simple release build
meson(ctx, ["--buildtype=release"])

# With additional options
meson(ctx, ["--buildtype=release", "-Dgtk_doc=false"])
```

### cmake(ctx, args?)
Build projects using CMake. The builder automatically configures the install prefix.

```starlark
cmake(ctx, [
    "-DCMAKE_BUILD_TYPE=Release",
    "-GNinja"  # Use Ninja generator
])

# With additional options
cmake(ctx, [
    "-DCMAKE_BUILD_TYPE=Release",
    "-DBUILD_SHARED_LIBS=ON",
    "-DWITH_SSL=ON"
])
```

### configure(ctx, args?)
Run configure script (typically for autotools projects). The builder automatically sets the prefix.

```starlark
configure(ctx)  # Basic configure
configure(ctx, ["--with-ssl", "--enable-shared"])  # With options
```

### make(ctx, args?)
Run make. The builder handles parallel jobs automatically.

```starlark
# After configure
make(ctx)

# With specific target
make(ctx, ["check"])  # Run tests
```

### autotools(ctx, args?)
Complete autotools workflow (configure, make, make install). The builder handles prefix configuration and automatically runs the full build and install process.

**Important**: `autotools()` includes the make and make install steps automatically. Do not call `make(ctx, ["install"])` separately.

```starlark
autotools(ctx)  # Basic autotools build
autotools(ctx, ["--enable-shared", "--disable-static"])  # With options
```

### go(ctx, args?)
Build Go projects. The builder handles binary installation automatically.

```starlark
# Download dependencies first
go(ctx, ["mod", "download"])

# Build
go(ctx, ["build", "./cmd/myapp"])

# Run tests
go(ctx, ["test", "./..."])
```

### python(ctx, args?)
Build Python projects. The builder manages installation paths automatically.

```starlark
# Using setup.py
python(ctx, ["setup.py", "build"])
python(ctx, ["setup.py", "install"])

# Using pip
python(ctx, ["-m", "pip", "install", "."])
```

### nodejs(ctx, args?)
Build Node.js projects.

```starlark
# npm
nodejs(ctx, ["npm", "ci"])
nodejs(ctx, ["npm", "run", "build"])

# yarn
nodejs(ctx, ["yarn", "install", "--frozen-lockfile"])
nodejs(ctx, ["yarn", "build"])

# pnpm
nodejs(ctx, ["pnpm", "install", "--frozen-lockfile"])
nodejs(ctx, ["pnpm", "run", "build"])
```

## Real-World Examples

### C
```starlark
#
# sps2 build recipe for curl
#
# This recipe builds curl from the latest source in its Git repository.
# It enables support for OpenSSL, zlib, and nghttp2 (for HTTP/2).
#

def metadata():
    """Return package metadata."""
    return {
        "name": "curl",
        "version": "8.14.1",
        "description": "A command-line tool and library for transferring data with URL syntax.",
        "license": "CUSTOM",  # MIT-like license, see LICENSES/curl.txt
        "homepage": "https://curl.se",
        "depends": [
            "openssl",
            "zlib",
            "nghttp2",
            "brotli",
            "libssh2",
            "libidn2",
            "libpsl",
        ],
        "build_depends": []
    }

def build(ctx):
    """Build the package using the provided context."""
    cleanup(ctx)

    # Apply optimized default compiler flags for macOS ARM64
    with_defaults(ctx)

    # 2. Fetch the source code from the official Git repository
    fetch(ctx, "https://github.com/curl/curl/releases/download/curl-8_14_1/curl-8.14.1.tar.bz2")

    # 3. Configure the build using CMake.
    # The sps2 `cmake` function handles the complete configure, build,
    # and packaging process. The installation prefix is set automatically.
    cmake(ctx, [
        # Standard release build flags
        "-DCMAKE_BUILD_TYPE=Release",
        "-GNinja",

        # Build shared libraries, which is common for system packages
        "-DBUILD_SHARED_LIBS=ON",

        # Explicitly disable building static libs to save time and space
        "-DBUILD_STATIC_LIBS=OFF",

        # Enable essential features
        "-DCURL_USE_OPENSSL=ON",
        "-DCURL_ZLIB=ON",
        "-DUSE_NGHTTP2=ON",      # For HTTP/2 support
        "-DENABLE_IPV6=ON",      # Enable IPv6 support
        "-DCURL_USE_LIBSSH2=ON", # SSH support
        "-DUSE_LIBIDN2=ON",      # International domain names
        "-DCURL_BROTLI=ON",      # Brotli compression
        "-DCURL_USE_LIBPSL=ON",  # Public suffix list

        # Disable features not typically needed for a runtime package
        "-DBUILD_TESTING=OFF",
        "-DENABLE_CURL_MANUAL=OFF",
    ])

    # Patch @rpath reference in binaries with hardcoded paths
    patch_rpaths(ctx) # should be avoided if not required, curl won't work without it

    # 4. (Optional) Install the package to the system prefix after a
    # successful build.
    # install(ctx)
```

### Rust Application (ripgrep)

```starlark
def metadata():
    """Package metadata"""
    return {
        "name": "ripgrep",
        "version": "14.1.1",
        "description": """ripgrep is a line-oriented search tool that recursively searches the current
                        directory for a regex pattern while respecting gitignore rules. ripgrep has
                        first class support on Windows, macOS and Linux.""",
        "license": "MIT",
        "homepage": "https://github.com/BurntSushi/ripgrep"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)

    # Clone git repository
    git(ctx, "https://github.com/BurntSushi/ripgrep", "HEAD")

    # Allow network access for dependency downloads
    allow_network(ctx, True)

    # Build using cargo build system
    cargo(ctx, ["--release"])
```

### Rust Editor (helix)

```starlark
def metadata():
    """Package metadata"""
    return {
        "name": "helix",
        "version": "25.1.1",
        "description": "A post-modern modal text editor.",
        "license": "MIT"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)

    # Clone git repository
    git(ctx, "https://github.com/helix-editor/helix", "HEAD")

    # Allow network access for dependency downloads
    allow_network(ctx, True)

    # Build using cargo build system
    cargo(ctx, ["--release"])
```

### C Library (pkgconf)

```starlark
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

    # Clone git repository
    git(ctx, "https://github.com/pkgconf/pkgconf", "HEAD")

    # Build using meson build system
    meson(ctx, ["--buildtype=release"])
```

## Advanced Features

### Feature Management

#### enable_feature(ctx, name)
Enable a build feature.
```starlark
enable_feature(ctx, "ssl")
enable_feature(ctx, "gui")
```

#### disable_feature(ctx, name)
Disable a build feature.
```starlark
disable_feature(ctx, "tests")
disable_feature(ctx, "docs")
```

#### with_features(ctx, features, callback)
Execute steps conditionally based on features.
```starlark
with_features(ctx, ["ssl", "zlib"], lambda: [
    configure(ctx, ["--with-ssl", "--with-zlib"]),
    make(ctx)
])
```

### Error Recovery

#### on_error(ctx, handler)
Register an error handler.
```starlark
on_error(ctx, "retry")
on_error(ctx, "skip_tests")
```

#### checkpoint(ctx, name)
Create a recovery checkpoint.
```starlark
checkpoint(ctx, "after_configure")
checkpoint(ctx, "before_tests")
```

#### try_recover(ctx, strategy, callback)
Try steps with recovery strategy.
```starlark
try_recover(ctx, "retry", lambda: [
    configure(ctx, ["--host=aarch64-apple-darwin"]),
    make(ctx)
])  # Strategies: retry, continue, abort
```

### Cross-Compilation - (Sps2 is only for macos on arm, we allow this anyway)

#### set_target(ctx, triple)
Set target triple for cross-compilation.
```starlark
set_target(ctx, "aarch64-apple-darwin")
set_target(ctx, "x86_64-unknown-linux-gnu")
```

#### set_toolchain(ctx, name, path)
Set toolchain component path.
```starlark
set_toolchain(ctx, "CC", "/usr/bin/clang")
set_toolchain(ctx, "CXX", "/usr/bin/clang++")
set_toolchain(ctx, "AR", "/usr/bin/ar")
```

### Parallel Execution

#### set_parallelism(ctx, jobs)
Set the parallelism level for builds.
```starlark
set_parallelism(ctx, 4)
set_parallelism(ctx, ctx.JOBS * 2)
```

#### parallel_steps(ctx, callback)
Execute multiple build steps in parallel.
```starlark
parallel_steps(ctx, lambda: [
    command(ctx, "make -C lib1"),
    command(ctx, "make -C lib2"),
    command(ctx, "make -C lib3")
])
```

#### set_resource_hints(ctx, cpu?, memory_mb?)
Provide resource hints for the build.
```starlark
set_resource_hints(ctx, cpu=4)           # Hint: needs 4 CPU cores
set_resource_hints(ctx, memory_mb=8192)  # Hint: needs 8GB RAM
set_resource_hints(ctx, cpu=8, memory_mb=16384)  # Both hints
```

## Important Notes

1. **Use draft for recipes** - Run `sps2 draft` to generate recipes automatically rather than writing from scratch.
2. **Minimal manual commands** - The builder handles packaging automatically. Avoid manual `mkdir`, `cp`, `mv` commands.
3. **Always cleanup first** - Start with `cleanup(ctx)` to ensure a clean build environment.
4. **Use git when possible** - `git()` is preferred over `fetch()` for better reproducibility.
5. **Hash validation is optional** - Use `fetch()` for convenience or `fetch_md5()`, `fetch_sha256()`, `fetch_blake3()` when hash validation is needed.
6. **Enable network judiciously** - Only enable network when needed for dependency downloads.
7. **No prefix configuration needed** - The builder automatically configures the correct prefix for all build systems.
8. **No direct file manipulation** - Let the build systems handle file installation.
9. **install() is optional** - Only use if you want automatic installation after build.

## Build Systems Implementation Status

### Fully Exposed in Starlark API:
- ✅ autotools (autotools, configure functions)
- ✅ cmake (cmake function)
- ✅ meson (meson function)
- ✅ cargo (cargo function)
- ✅ make (make function)
- ✅ go (go function)
- ✅ python (python function)
- ✅ nodejs (nodejs function)

## Best Practices Summary

1. Use `sps2 draft` to generate initial recipes from source repositories
2. Start builds with `cleanup(ctx)`
3. Use `git()` or `fetch()` to get source (or `fetch_md5()`, `fetch_sha256()`, `fetch_blake3()` for hash validation)
4. Enable network if needed with `allow_network(ctx, True)`
5. Call the appropriate build system function
6. Let the builder handle packaging automatically
7. Optionally add `install(ctx)` at the end for auto-installation
