# sps2 YAML Build Script Documentation

## Overview

sps2 uses YAML format for build recipes. Each package recipe is a `.yaml` file that defines metadata and build instructions using a declarative, staged approach. The builder automatically handles packaging - you just need to specify how to build the software.

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

Every YAML recipe uses a staged execution model with these sections:

```yaml
# Package metadata (required)
metadata:
  name: package-name      # Required: package name
  version: "1.0.0"        # Required: package version
  description: "..."      # Optional: package description
  homepage: "..."         # Optional: project homepage
  license: "MIT"          # Optional: license identifier
  runtime_deps: []        # Optional: runtime dependencies
  build_deps: []          # Optional: build-time dependencies

# Dynamic variables (optional)
facts:
  MY_VAR: "value"         # Define custom variables

# Build environment configuration (optional)
environment:
  isolation: default      # Isolation level: none|default|enhanced|hermetic
  defaults: true          # Apply optimized compiler flags
  network: false          # Allow network access during build
  variables:              # Additional environment variables
    KEY: "value"

# Source acquisition (required)
source:
  fetch:                  # Fetch and extract source archive
    url: "https://..."
    checksum: "sha256:..." # Optional but recommended
  # OR
  git:                    # Clone from git repository
    url: "https://..."
    ref: "v1.0.0"         # Tag, branch, or commit
  # OR
  local:                  # Copy from local directory
    path: "./src"
  
  patches: []             # Optional: patches to apply

# Build instructions (required)
build:
  system: autotools       # Use a build system preset
  # OR
  steps:                  # Custom build commands
    - make -j${JOBS}

# Post-processing (optional)
post:
  patch_rpaths: default   # Fix library paths (default/absolute/skip)
  fix_permissions: true   # Fix executable permissions

# Installation (optional)
install:
  auto: true              # Auto-install after build
```

## Execution Stages

The YAML format enforces proper staged execution:

1. **Environment Stage**: Isolation, defaults, variables are applied
2. **Source Stage**: Fetch/git/local operations, then patches
3. **Build Stage**: Build system or custom commands execute
4. **Post Stage**: Post-processing like rpath patching
5. **Validation**: Automatic validation and fixes
6. **Package**: Create .sp package file

## Metadata Section

```yaml
metadata:
  name: curl              # Package name (required)
  version: "8.14.1"       # Version string (required)
  description: "..."      # One-line description
  homepage: "https://..." # Project website
  license: "MIT"          # SPDX license identifier
  runtime_deps:           # Runtime dependencies
    - openssl
    - zlib
  build_deps:             # Build-only dependencies
    - cmake
    - ninja
```

## Facts and Variables

Facts allow dynamic values in your recipe:

```yaml
facts:
  # Built-in facts (automatically available)
  # ${NAME} - Package name from metadata
  # ${VERSION} - Package version from metadata
  # ${PREFIX} - Installation prefix (/opt/pm/live)
  # ${JOBS} - Parallel job count
  
  # Custom facts
  CONFIGURE_ARGS: "--enable-shared --disable-static"
  PYTHON_VERSION: "3.11"

# Use facts with ${} syntax
source:
  fetch:
    url: "https://example.com/${NAME}-${VERSION}.tar.gz"

build:
  steps:
    - ./configure ${CONFIGURE_ARGS}
```

## Environment Section

```yaml
environment:
  # Isolation level (default: default)
  # - none: No isolation (not recommended)
  # - default: Standard isolation (default)
  # - enhanced: Private HOME/TMPDIR
  # - hermetic: Full isolation
  isolation: default
  
  # Apply optimized compiler flags (default: false)
  # Sets -O2, -mcpu=apple-m1, security flags, etc.
  defaults: true
  
  # Allow network access (default: false)
  # Enable for cargo, go, npm, etc.
  network: true
  
  # Additional environment variables
  variables:
    CUSTOM_FLAG: "value"
    BUILD_TYPE: "Release"
```

## Source Section

### Fetch from URL
```yaml
source:
  fetch:
    url: "https://github.com/curl/curl/releases/download/curl-8_14_1/curl-8.14.1.tar.bz2"
    # Checksum is optional but recommended
    checksum: "sha256:abc123..."  # Supports sha256, blake3, md5
```

### Clone from Git
```yaml
source:
  git:
    url: "https://github.com/BurntSushi/ripgrep"
    ref: "14.1.1"  # Tag, branch, or commit SHA
```

### Copy Local Files
```yaml
source:
  local:
    path: "./my-source"  # Relative to recipe directory
```

### Apply Patches
```yaml
source:
  fetch:
    url: "..."
  patches:
    - fix-macos-build.patch
    - security-fix.patch
```

## Build Section

### Using Build System Presets

Build systems automatically handle configure, build, and install:

```yaml
build:
  system: autotools       # ./configure && make && make install
  # OR
  system: cmake          # cmake && cmake --build && cmake --install
  args: ["-DCMAKE_BUILD_TYPE=Release"]
  # OR
  system: meson          # meson setup && meson compile && meson install
  # OR  
  system: cargo          # cargo build --release && install binaries
  args: ["--features", "ssl"]
  # OR
  system: go             # go build && install binaries
  # OR
  system: python         # python setup.py or pip install
  # OR
  system: nodejs         # npm/yarn/pnpm install && build
```

### Custom Build Steps

```yaml
build:
  steps:
    # Commands are executed in order
    - ./configure --prefix=${PREFIX}
    - make -j${JOBS}
    - make check
    - make install DESTDIR=${DESTDIR}
```

### Build System Reference

| System | Description | Automatic Actions |
|--------|-------------|-------------------|
| `autotools` | GNU Autotools | `./configure --prefix=/opt/pm/live && make -j && make install` |
| `cmake` | CMake builds | `cmake -DCMAKE_INSTALL_PREFIX=/opt/pm/live && cmake --build . && cmake --install` |
| `meson` | Meson/Ninja | `meson setup --prefix=/opt/pm/live && meson compile && meson install` |
| `cargo` | Rust/Cargo | `cargo build --release` + binary collection |
| `go` | Go modules | `go build` + binary installation |
| `python` | Python packages | `pip install` or `setup.py install` |
| `nodejs` | Node.js packages | `npm/yarn/pnpm install && build` |
| `make` | Plain Makefile | `make -j && make install` |

## Security and Validation

### Dual-Layer Security Architecture

sps2 implements a **dual-layer validation system** to ensure build scripts are safe to execute:

```
YAML Recipe → ParsedStep → [Recipe Validation] → BuildStep → [Security Context] → Execution
                               ↑                                    ↑
                               |                                    |
                          Static checks                      Runtime checks
                          at parse time                      with full context
```

1. **Recipe-Time Validation** (Static)
   - Validates commands when parsing the YAML recipe
   - Checks against allowed commands list from `config.toml`
   - Blocks dangerous patterns (sudo, rm -rf, etc.)
   - Cannot expand variables yet (no runtime context)

2. **Runtime Security Context** (Stateful)
   - Validates during actual command execution
   - Expands variables like `${DESTDIR}`, `${PREFIX}`, `${BUILD_DIR}`
   - Tracks current directory across command chains
   - Ensures all paths stay within `/opt/pm/build/`
   - Detects symlink attacks and path traversal

### Configuring Allowed Commands

Build commands must be explicitly allowed in your configuration file:

**Location**: `~/.config/sps2/config.toml`

```toml
[build_script_allowed_commands]
allowed = [
    # Build tools
    "make", "cmake", "meson", "ninja", "cargo", "go",
    
    # Compilers
    "gcc", "g++", "clang", "clang++", "cc", "c++",
    
    # Common utilities
    "cp", "mv", "rm", "mkdir", "ln", "chmod", "install",
    "sed", "awk", "grep", "find", "patch", "echo", "cat",
    
    # Package/archive tools
    "tar", "gzip", "bzip2", "xz", "unzip", "zip",
    
    # Add your custom commands here:
    # "python3",
    # "npm",
]

# Explicitly disallowed (even if in allowed list)
disallowed = [
    "sudo", "doas", "su",           # No privilege escalation
    "systemctl", "service",         # No system services
    "apt", "yum", "dnf", "pacman",  # No system package managers
]
```

To allow additional commands:
1. Edit `~/.config/sps2/config.toml`
2. Add to the `allowed` list
3. Save and retry your build

### Security Best Practices

1. **Always use build variables** instead of hardcoded paths:
   ```yaml
   # Good: Uses proper variables
   - make install DESTDIR=${DESTDIR} PREFIX=${PREFIX}
   
   # Bad: Hardcoded system paths
   - make install PREFIX=/usr/local
   ```

2. **Commands are validated individually** in multi-line scripts:
   ```yaml
   - shell: |
       cd ${BUILD_DIR}          # ✓ Validated
       ./configure              # ✓ Must be in allowed list
       make -j${JOBS}          # ✓ Each command checked
       sudo make install       # ✗ Blocked: sudo not allowed
   ```

3. **Path containment** is enforced:
   ```yaml
   # These will be blocked:
   - cd /etc                    # Outside build root
   - rm -rf /                   # Dangerous pattern
   - ln -s /etc/passwd passwd   # System file access
   
   # These are allowed:
   - cd ${BUILD_DIR}/src        # Within build root
   - rm -f build/*.o            # Specific file removal
   - ln -s libfoo.so.1 libfoo.so # Library symlinks
   ```

### Common Security Errors

**"Command 'X' is not in the allowed commands list"**
- Add the command to `~/.config/sps2/config.toml` allowed list

**"Path escape attempt: X resolves to Y outside build root"**
- Ensure paths stay within `${BUILD_DIR}` or use `${DESTDIR}`

**"rm -rf is not allowed in build scripts"**
- Use targeted removal: `rm -f specific_file` or `rm -r specific_dir`

**"cd without arguments not allowed"**
- Always specify a path: `cd ${BUILD_DIR}`

### Build Variables

These variables are expanded at runtime:

| Variable | Description | Example Value |
|----------|-------------|---------------|
| `${DESTDIR}` | Staging directory for installation | `/opt/pm/build/curl-8.14.1/destdir` |
| `${PREFIX}` | Installation prefix | `/opt/pm/live` |
| `${BUILD_DIR}` | Current build directory | `/opt/pm/build/curl-8.14.1` |
| `${PACKAGE_NAME}` | Package name from metadata | `curl` |
| `${PACKAGE_VERSION}` | Package version from metadata | `8.14.1` |
| `${JOBS}` | Parallel job count | `8` |

### Debugging Security Issues

Run with debug logging to see validation details:
```bash
sps2 --debug build package-name
```

This will show:
- Which commands are being validated
- How paths are being resolved
- What the security context is tracking

## Post-Processing Section

```yaml
post:
  # Fix library paths (default behavior is modern @rpath)
  patch_rpaths: default    # default: keep @rpath references (relocatable)
                          # absolute: convert to absolute paths (homebrew-style)
                          # skip: disable rpath patching entirely
  
  # Override automatic QA pipeline selection (advanced users)
  qa_pipeline: auto        # auto: automatic detection based on build system
                          # rust: minimal validation for Rust binaries
                          # c: full validation for C/C++ binaries  
                          # go: medium validation for Go binaries
                          # python: light validation for Python packages
                          # skip: disable artifact QA entirely (dangerous!)
  
  # Fix executable permissions (rarely needed)
  fix_permissions: true    # true/false to fix all executables
  # OR specify paths:
  fix_permissions:
    - bin/
    - libexec/
  
  # Custom post-processing commands (always run as shell)
  commands:
    - strip bin/*    # Strip debug symbols
    - find lib -name '*.la' -delete  # Remove libtool files
```

### When to Use Post-Processing

- **patch_rpaths**: 
  - `default` (or omit): Modern @rpath style - recommended for most packages
  - `absolute`: Use when binaries fail with "dylib not found" errors or tools don't understand @rpath
  - `skip`: Use for packages that manage their own rpaths or don't have dynamic libraries

- **qa_pipeline**: Override automatic build system detection
  - `auto` (or omit): Automatic detection based on build commands used
  - `rust`: For Rust packages built with custom shell scripts instead of cargo
  - `c`: Force full C/C++ validation pipeline for non-standard builds
  - `go`: For Go packages built with custom commands
  - `python`: For Python packages built with custom commands  
  - `skip`: **Only for debugging** - disables all validation and patching

- **fix_permissions**: Only needed when installed binaries lack execute permissions (some packages like GCC)

- The default behavior (when `post:` is omitted) applies modern rpath patching and automatic QA pipeline selection

## Installation Section

```yaml
install:
  auto: true    # Automatically install after building
                # false = only build the .sp package
```

## Real-World Examples

### C/C++ with CMake
```yaml
metadata:
  name: curl
  version: "8.14.1"
  description: "Command-line tool for transferring data with URLs"
  license: "MIT"
  homepage: "https://curl.se"
  runtime_deps: [openssl, zlib, nghttp2, brotli]

environment:
  defaults: true    # Optimized flags for macOS ARM64

source:
  fetch:
    url: "https://github.com/curl/curl/releases/download/curl-8_14_1/curl-8.14.1.tar.bz2"

build:
  system: cmake
  args:
    - "-DCMAKE_BUILD_TYPE=Release"
    - "-DBUILD_SHARED_LIBS=ON"
    - "-DCURL_USE_OPENSSL=ON"
    - "-DCURL_ZLIB=ON"
    - "-DUSE_NGHTTP2=ON"

post:
  patch_rpaths: absolute  # Use absolute paths for curl compatibility
```

### Rust Application
```yaml
metadata:
  name: ripgrep
  version: "14.1.1"
  description: "Line-oriented search tool"
  license: "MIT"
  homepage: "https://github.com/BurntSushi/ripgrep"

environment:
  network: true     # Cargo needs network for dependencies

source:
  git:
    url: "https://github.com/BurntSushi/ripgrep"
    ref: "14.1.1"

build:
  system: cargo
  args: ["--release"]
```

### Rust with Custom Build (requires qa_pipeline override)
```yaml
metadata:
  name: rust
  version: "1.88.0"
  description: "Rust compiler and toolchain"
  license: "MIT"

source:
  sources:
    - fetch:
        url: "https://static.rust-lang.org/dist/rustc-1.88.0-src.tar.gz"
        extract_to: "src"
    - fetch:
        url: "https://static.rust-lang.org/dist/rust-1.87.0-aarch64-apple-darwin.tar.gz"
        extract_to: "bootstrap"

build:
  steps:
    - shell: |
        cd ../bootstrap && ./install.sh --prefix=/tmp/rust-bootstrap
        cd ../src && echo '[build]' > config.toml
        echo 'rustc = "/tmp/rust-bootstrap/bin/rustc"' >> config.toml
        python3 x.py build --config config.toml
        python3 x.py install --config config.toml

post:
  qa_pipeline: rust    # Override: shell commands look like C/C++, but this is Rust
  patch_rpaths: skip   # Rust manages its own library paths
```

### Python Package
```yaml
metadata:
  name: mypy
  version: "1.11.0"
  description: "Optional static typing for Python"
  license: "MIT"
  runtime_deps: [python@3.11]

environment:
  network: true

source:
  fetch:
    url: "https://github.com/python/mypy/archive/v1.11.0.tar.gz"

build:
  system: python
```

### Simple Make-based Project
```yaml
metadata:
  name: htop
  version: "3.3.0"
  description: "Interactive process viewer"
  license: "GPL-2.0"

source:
  fetch:
    url: "https://github.com/htop-dev/htop/releases/download/3.3.0/htop-3.3.0.tar.xz"

build:
  system: autotools
```

### Complex Build (GCC Compiler)
```yaml
metadata:
  name: gcc
  version: "15.1.0"
  description: "GNU Compiler Collection"
  license: "GPL-3.0-or-later"
  dependencies:
    build: [gmp, mpfr, mpc, isl, zstd]

facts:
  build_triple: "aarch64-apple-darwin24"
  sdk_path: "/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk"

environment:
  defaults: true
  variables:
    BOOT_LDFLAGS: "-Wl,-headerpad_max_install_names -Wl,-rpath,${PREFIX}/lib"

source:
  local:
    path: "."
  patches:
    - "gcc-15.1.0-darwin.patch"

build:
  steps:
    # Complex out-of-source build requiring shell features
    - shell: |
        mkdir -p build
        cd build && ../configure \
          --prefix=${PREFIX} \
          --build=${build_triple} \
          --with-sysroot=${sdk_path} \
          --with-native-system-header-dir=/usr/include \
          --with-gmp=${PREFIX} \
          --with-mpfr=${PREFIX} \
          --with-mpc=${PREFIX} \
          --with-isl=${PREFIX} \
          --with-zstd=${PREFIX} \
          --enable-languages=c,c++,objc,obj-c++,fortran \
          --disable-multilib \
          --enable-checking=release \
          --with-gcc-major-version-only \
          --with-system-zlib \
          --disable-nls \
          --enable-bootstrap
    
    # Build with special flags
    - shell: |
        cd build && make -j8 BOOT_LDFLAGS="${BOOT_LDFLAGS}"
    
    # Install
    - shell: |
        cd build && make install

post:
  fix_permissions: true
```

## Best Practices

1. **Use build system presets** - They handle all the complex details automatically
2. **Enable compiler defaults** - `environment.defaults: true` for optimized builds
3. **Specify checksums** - Always include checksums for fetched sources
4. **Avoid manual commands** - Let build systems handle installation paths
5. **Skip post-processing** - Most packages don't need rpath or permission fixes
6. **Use facts for flexibility** - Define reusable values as facts
7. **Test locally first** - Use `sps2 build recipe.yaml` to test

## Migration from Starlark

| Starlark | YAML Equivalent |
|----------|-----------------|
| `cleanup(ctx)` | Automatic before source stage |
| `fetch(ctx, url)` | `source.fetch.url` |
| `git(ctx, url, ref)` | `source.git` |
| `with_defaults(ctx)` | `environment.defaults: true` |
| `allow_network(ctx, true)` | `environment.network: true` |
| `autotools(ctx)` | `build.system: autotools` |
| `patch_rpaths(ctx)` | `post.patch_rpaths` |
| `fix_permissions(ctx)` | `post.fix_permissions` |
| `install(ctx)` | `install.auto: true` |

## Troubleshooting

**Build fails with "command not found"**
- Make sure build dependencies are listed in `metadata.build_deps`

**Network errors during build**
- Set `environment.network: true` for packages that download dependencies

**Installed binaries won't run**
- Try `post.patch_rpaths: absolute` for compatibility with tools that don't understand @rpath

**Permission denied when running installed programs**
- Add `post.fix_permissions: [bin/]`

**Rust/Go binaries panic or behave strangely after building**
- Add `post.qa_pipeline: rust` (or `go`) if you used shell commands instead of the proper build system
- The automatic detection may have applied C/C++ binary patching that breaks language-specific runtimes

**Want to see which QA pipeline is being used**
- Run with `sps2 build --debug` and look for "Using [pipeline] for build systems" messages

**Need to see what's happening**
- Run with `RUST_LOG=debug sps2 build recipe.yaml`