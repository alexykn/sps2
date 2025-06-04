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

## Build Methods

### Basic Build Operations

#### ctx.fetch(url)
Fetch and extract source archive.
```starlark
ctx.fetch("https://example.com/package-1.0.0.tar.gz")
```

#### ctx.command(cmd)
Execute arbitrary shell command.
```starlark
ctx.command("mkdir -p build")
ctx.command("echo 'Hello' > README")
```

#### ctx.apply_patch(path)
Apply a patch file to the source.
```starlark
ctx.apply_patch("fix-build.patch")
```

#### ctx.install()
Run the install phase (typically `make install`).
```starlark
ctx.install()
```

### Build System Methods

#### ctx.make(args=[])
Run make with optional arguments.
```starlark
ctx.make()                           # Simple make
ctx.make(["-j" + str(ctx.JOBS)])   # Parallel make
ctx.make(["install", "PREFIX=/usr"]) # Make with target and vars
```

#### ctx.configure(args=[])
Run configure script (typically for autotools).
```starlark
ctx.configure()                                          # Basic configure
ctx.configure(["--prefix=" + ctx.PREFIX, "--disable-static"])  # With options
```

#### ctx.autotools(args=[])
Run full autotools sequence (configure && make && make install).
```starlark
ctx.autotools()
ctx.autotools(["--enable-shared", "--disable-static"])
```

#### ctx.cmake(args=[])
Run CMake build.
```starlark
ctx.cmake(["-DCMAKE_INSTALL_PREFIX=" + ctx.PREFIX, 
           "-DCMAKE_BUILD_TYPE=Release"])
```

#### ctx.meson(args=[])
Run Meson build.
```starlark
ctx.meson(["--prefix=" + ctx.PREFIX, "--buildtype=release"])
```

#### ctx.cargo(args=[])
Run Cargo (Rust) build.
```starlark
ctx.cargo(["build", "--release"])
```

### Build System Detection

#### ctx.detect_build_system()
Automatically detect the build system.
```starlark
ctx.detect_build_system()
```

#### ctx.set_build_system(name)
Manually set the build system.
```starlark
ctx.set_build_system("cmake")  # Options: autotools, cmake, meson, cargo, go, python, nodejs
```

### Feature Management

#### ctx.enable_feature(name)
Enable a build feature.
```starlark
ctx.enable_feature("ssl")
ctx.enable_feature("gui")
```

#### ctx.disable_feature(name)
Disable a build feature.
```starlark
ctx.disable_feature("tests")
ctx.disable_feature("docs")
```

#### ctx.with_features(features, steps)
Execute steps conditionally based on features.
```starlark
# Note: Currently has limited implementation
ctx.with_features(["ssl", "zlib"], [
    # Steps to execute if features are enabled
])
```

### Error Recovery

#### ctx.on_error(handler)
Register an error handler.
```starlark
ctx.on_error("retry")
ctx.on_error("skip_tests")
```

#### ctx.checkpoint(name)
Create a recovery checkpoint.
```starlark
ctx.checkpoint("after_configure")
ctx.checkpoint("before_tests")
```

#### ctx.try_recover(steps, strategy)
Try steps with recovery strategy.
```starlark
# Note: Currently has limited implementation
ctx.try_recover([], "retry")  # Strategies: retry, continue, abort
```

### Cross-Compilation

#### ctx.set_target(triple)
Set target triple for cross-compilation.
```starlark
ctx.set_target("aarch64-apple-darwin")
ctx.set_target("x86_64-unknown-linux-gnu")
```

#### ctx.set_toolchain(name, path)
Set toolchain component path.
```starlark
# Note: Currently only stores first argument due to Starlark limitations
ctx.set_toolchain("CC", "/usr/bin/clang")
ctx.set_toolchain("CXX", "/usr/bin/clang++")
```

### Parallel Execution

#### ctx.set_parallelism(jobs)
Set the parallelism level for builds.
```starlark
ctx.set_parallelism(4)
ctx.set_parallelism(ctx.JOBS * 2)
```

#### ctx.parallel_steps(steps)
Execute multiple build steps in parallel.
```starlark
# Note: Currently has limited implementation
ctx.parallel_steps([
    # List of steps to run in parallel
])
```

#### ctx.set_resource_hints(cpu=None, memory_mb=None)
Provide resource hints for the build.
```starlark
# Note: Currently has simplified implementation
ctx.set_resource_hints()  # Will be enhanced in future
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
    ctx.command("mkdir -p src")
    ctx.command("cat > src/hello.c << 'EOF'\n#include <stdio.h>\nint main() { printf(\"Hello!\\n\"); return 0; }\nEOF")
    
    # Create Makefile
    ctx.command("echo 'hello: src/hello.c' > Makefile")
    ctx.command("echo '\t$(CC) -o hello src/hello.c' >> Makefile")
    
    # Build
    ctx.make(["-j" + str(ctx.JOBS)])
    
    # Install
    ctx.command("mkdir -p stage" + ctx.PREFIX + "/bin")
    ctx.command("cp hello stage" + ctx.PREFIX + "/bin/")
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
    ctx.fetch("https://example.com/app-2.0.0.tar.gz")
    
    # Configure with CMake
    ctx.cmake([
        "-DCMAKE_INSTALL_PREFIX=" + ctx.PREFIX,
        "-DCMAKE_BUILD_TYPE=Release",
        "-DWITH_SSL=ON",
        "-S", ".",
        "-B", "build"
    ])
    
    # Build
    ctx.command("cd build")
    ctx.make(["-C", "build", "-j" + str(ctx.JOBS)])
    
    # Test
    ctx.make(["-C", "build", "test"])
    
    # Install
    ctx.make(["-C", "build", "install", "DESTDIR=$(pwd)/stage"])
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
    ctx.fetch("https://gnu.org/app-3.5.0.tar.gz")
    
    # Enable optional features
    ctx.enable_feature("nls")
    ctx.enable_feature("shared")
    ctx.disable_feature("static")
    
    # Configure with features
    ctx.configure([
        "--prefix=" + ctx.PREFIX,
        "--enable-nls",
        "--enable-shared",
        "--disable-static"
    ])
    
    # Build with parallel jobs
    ctx.make(["-j" + str(ctx.JOBS)])
    
    # Install
    ctx.install()
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
    ctx.fetch("https://example.com/app-1.0.0.tar.gz")
    
    # Set cross-compilation target
    ctx.set_target("aarch64-linux-gnu")
    
    # Set toolchain
    ctx.set_toolchain("CC", "aarch64-linux-gnu-gcc")
    
    # Configure for cross-compilation
    ctx.configure([
        "--host=aarch64-linux-gnu",
        "--prefix=" + ctx.PREFIX
    ])
    
    ctx.make(["-j" + str(ctx.JOBS)])
    ctx.install()
```

## Build Systems Implementation Status

### Fully Exposed in Starlark API:
- ✅ autotools (ctx.autotools, ctx.configure)
- ✅ cmake (ctx.cmake)
- ✅ meson (ctx.meson)
- ✅ cargo (ctx.cargo)
- ✅ make (ctx.make)

### Implemented in Builder but NOT Exposed:
- ❌ go - Use `ctx.command("go build")` as workaround
- ❌ python - Use `ctx.command("python setup.py")` as workaround
- ❌ nodejs - Use `ctx.command("npm install")` as workaround

## Important Notes

1. **No println!** - All output goes through the Event channel, not stdout
2. **File paths** - Always use absolute paths or paths relative to build directory
3. **Staging** - Install to `stage$(ctx.PREFIX)` not directly to `ctx.PREFIX`
4. **Dependencies** - Use semantic versioning in metadata dependencies
5. **Error handling** - Methods currently don't return errors but will in future versions

## Limitations

Due to Starlark 0.13 limitations:
- Methods with multiple arguments may have simplified implementations
- List/array arguments are not fully supported in some methods
- Some advanced features like `with_features` and `parallel_steps` have placeholder implementations

## Future Enhancements

Planned improvements:
- Full argument support for all methods
- Native Go, Python, and Node.js build system methods
- Enhanced error recovery mechanisms
- Better resource management for parallel builds
- BLAKE3 hash verification in fetch()