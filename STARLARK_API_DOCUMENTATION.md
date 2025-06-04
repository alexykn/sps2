# spsv2 Starlark Build Script API Documentation

## Overview

spsv2 uses Starlark (a Python-like language) for build scripts. Each package recipe is a `.star` file that defines metadata and build instructions.

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
- `ctx.PREFIX` - Installation prefix (e.g., `/opt/pm/live`)
- `ctx.JOBS` - Number of parallel build jobs (integer)

## Build Functions

### Basic Build Operations

#### fetch(ctx, url, hash?)
Fetch and extract source archive.
```starlark
fetch(ctx, "https://example.com/package-1.0.0.tar.gz")
fetch(ctx, "https://example.com/package-1.0.0.tar.gz", "blake3:abc123...")
```

#### command(ctx, cmd)
Execute arbitrary shell command. Can accept string or list.
```starlark
command(ctx, "mkdir -p build")
command(ctx, "echo 'Hello' > README")
command(ctx, ["gcc", "-o", "output", "input.c"])
```

#### apply_patch(ctx, path)
Apply a patch file to the source.
```starlark
apply_patch(ctx, "fix-build.patch")
```

#### install(ctx)
Run the install phase (typically `make install`).
```starlark
install(ctx)
```

### Build System Functions

#### make(ctx, args?)
Run make with optional arguments.
```starlark
make(ctx)                           # Simple make
make(ctx, ["-j" + str(ctx.JOBS)])   # Parallel make
make(ctx, ["install", "PREFIX=/usr"]) # Make with target and vars
```

#### configure(ctx, args?)
Run configure script (typically for autotools).
```starlark
configure(ctx)                                          # Basic configure
configure(ctx, ["--prefix=" + ctx.PREFIX, "--disable-static"])  # With options
```

#### autotools(ctx, args?)
Run full autotools sequence (configure && make && make install).
```starlark
autotools(ctx)
autotools(ctx, ["--enable-shared", "--disable-static"])
```

#### cmake(ctx, args?)
Run CMake build.
```starlark
cmake(ctx, ["-DCMAKE_INSTALL_PREFIX=" + ctx.PREFIX, 
           "-DCMAKE_BUILD_TYPE=Release"])
```

#### meson(ctx, args?)
Run Meson build.
```starlark
meson(ctx, ["--prefix=" + ctx.PREFIX, "--buildtype=release"])
```

#### cargo(ctx, args?)
Run Cargo (Rust) build.
```starlark
cargo(ctx, ["build", "--release"])
```

### Build System Detection

#### detect_build_system(ctx)
Automatically detect the build system.
```starlark
detect_build_system(ctx)
```

#### set_build_system(ctx, name)
Manually set the build system.
```starlark
set_build_system(ctx, "cmake")  # Options: autotools, cmake, meson, cargo, go, python, nodejs
```

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
# Execute callback if all features are enabled
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
# Execute callback with recovery strategy
try_recover(ctx, "retry", lambda: [
    configure(ctx, ["--host=aarch64-apple-darwin"]),
    make(ctx)
])  # Strategies: retry, continue, abort
```

### Cross-Compilation

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
# Execute steps in parallel
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

## Complete Examples

### Simple C Program
```starlark
def metadata():
    return {
        "name": "hello-world",
        "version": "1.0.0",
        "description": "Simple hello world program",
        "license": "MIT"
    }

def build(ctx):
    # Create source
    command(ctx, "mkdir -p src")
    command(ctx, "cat > src/hello.c << 'EOF'\n#include <stdio.h>\nint main() { printf(\"Hello!\\n\"); return 0; }\nEOF")
    
    # Create Makefile
    command(ctx, "echo 'hello: src/hello.c' > Makefile")
    command(ctx, "echo '\t$(CC) -o hello src/hello.c' >> Makefile")
    
    # Build
    make(ctx, ["-j" + str(ctx.JOBS)])
    
    # Install
    command(ctx, "mkdir -p stage" + ctx.PREFIX + "/bin")
    command(ctx, "cp hello stage" + ctx.PREFIX + "/bin/")
```

### CMake Project
```starlark
def metadata():
    return {
        "name": "cmake-app",
        "version": "2.0.0",
        "description": "Application built with CMake",
        "depends": ["libssl>=1.1.1", "zlib~=1.2"],
        "build_depends": ["cmake>=3.16", "gcc>=9.0"]
    }

def build(ctx):
    # Fetch source
    fetch(ctx, "https://example.com/app-2.0.0.tar.gz")
    
    # Configure with CMake
    cmake(ctx, [
        "-DCMAKE_INSTALL_PREFIX=" + ctx.PREFIX,
        "-DCMAKE_BUILD_TYPE=Release",
        "-DWITH_SSL=ON",
        "-S", ".",
        "-B", "build"
    ])
    
    # Build
    command(ctx, "cd build")
    make(ctx, ["-C", "build", "-j" + str(ctx.JOBS)])
    
    # Test
    make(ctx, ["-C", "build", "test"])
    
    # Install
    make(ctx, ["-C", "build", "install", "DESTDIR=$(pwd)/stage"])
```

### Autotools Project with Features
```starlark
def metadata():
    return {
        "name": "gnu-app",
        "version": "3.5.0",
        "description": "GNU application",
        "license": "GPL-3.0"
    }

def build(ctx):
    fetch(ctx, "https://gnu.org/app-3.5.0.tar.gz")
    
    # Enable optional features
    enable_feature(ctx, "nls")
    enable_feature(ctx, "shared")
    disable_feature(ctx, "static")
    
    # Configure with features
    configure(ctx, [
        "--prefix=" + ctx.PREFIX,
        "--enable-nls",
        "--enable-shared",
        "--disable-static"
    ])
    
    # Build with parallel jobs
    make(ctx, ["-j" + str(ctx.JOBS)])
    
    # Install
    install(ctx)
```

### Cross-Compilation Example
```starlark
def metadata():
    return {
        "name": "cross-app",
        "version": "1.0.0",
        "description": "Cross-compiled application"
    }

def build(ctx):
    fetch(ctx, "https://example.com/app-1.0.0.tar.gz")
    
    # Set cross-compilation target
    set_target(ctx, "aarch64-linux-gnu")
    
    # Set toolchain
    set_toolchain(ctx, "CC", "aarch64-linux-gnu-gcc")
    set_toolchain(ctx, "CXX", "aarch64-linux-gnu-g++")
    
    # Configure for cross-compilation
    configure(ctx, [
        "--host=aarch64-linux-gnu",
        "--prefix=" + ctx.PREFIX
    ])
    
    make(ctx, ["-j" + str(ctx.JOBS)])
    install(ctx)
```

## Build Systems Implementation Status

### Fully Exposed in Starlark API:
- ✅ autotools (autotools, configure functions)
- ✅ cmake (cmake function)
- ✅ meson (meson function)
- ✅ cargo (cargo function)
- ✅ make (make function)

### Implemented in Builder but NOT Exposed:
- ❌ go - Use `command(ctx, "go build")` as workaround
- ❌ python - Use `command(ctx, "python setup.py")` as workaround
- ❌ nodejs - Use `command(ctx, "npm install")` as workaround

## Important Notes

1. **No println!** - All output goes through the Event channel, not stdout
2. **File paths** - Always use absolute paths or paths relative to build directory
3. **Staging** - Install to `stage$(ctx.PREFIX)` not directly to `ctx.PREFIX`
4. **Dependencies** - Use semantic versioning in metadata dependencies
5. **Error handling** - Methods currently don't return errors but will in future versions

## API Changes in Latest Version

The Starlark API has been refactored to use global functions instead of methods:
- All build operations are now global functions that take `ctx` as the first parameter
- This change was made to work around Starlark 0.13's argument handling limitations
- The functionality remains the same, only the calling syntax has changed

## Future Enhancements

Planned improvements:
- Native Go, Python, and Node.js build system functions
- Enhanced error recovery mechanisms
- Better resource management for parallel builds
- BLAKE3 hash verification in fetch()
- More sophisticated conditional execution with with_features()
- Enhanced parallel_steps() implementation