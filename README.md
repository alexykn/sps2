# Project Overview
spsv2 is a modern package manager written in Rust, designed exclusively for macOS ARM64 (Apple Silicon). Built from the ground up with atomic operations, rollback capabilities, and a clean async architecture. This is a completely new package manager with its own package format, build system, and distribution infrastructure.

## Installation

### Prerequisites
- macOS with Apple Silicon (M1/M2/M3)
- Rust 1.86.0 or later
- SQLite 3.x
- sudo access for /opt/pm directory

### Setup
```bash
# Clone the repository
git clone https://github.com/yourusername/spsv2.git
cd spsv2

# Build the project
cargo build --release

# Run setup script (requires sudo)
sudo ./setup.sh

# Add to PATH in your shell config
echo 'export PATH="/opt/pm/live/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc

# Verify installation
sps2 --version
```

### SQLx Setup (for development)
The state crate uses SQLx compile-time checked queries. For development:
```bash
# Install sqlx-cli
cargo install sqlx-cli --no-default-features --features sqlite

# Set database URL
export DATABASE_URL="sqlite:///opt/pm/state.sqlite"

# Prepare offline queries (run in crates/state/)
cd crates/state
cargo sqlx prepare
```

## General Development Rules

### Rust Standards
- **Edition**: Rust 2021
- **Resolver**: Version 3 (set in workspace Cargo.toml)
- **MSRV**: 1.86.0 (latest stable)
- **Target**: `aarch64-apple-darwin` only

### Code Quality Requirements
1. **All code must pass `cargo fmt`** - No exceptions
2. **All code must pass `cargo clippy`** - With pedantic lints enabled
3. **No warnings allowed** - Use `#![deny(warnings)]` in lib.rs or enforce via CI with `cargo clippy -- -D warnings`
4. **Deny unsafe code by default** - Use `#![deny(clippy::pedantic, unsafe_code)]` in every lib.rs
5. **Unsafe code requires justification** - If needed, use `#[allow(unsafe_code)]` with detailed safety comment

### Best Practices
- Prefer `&str` over `String` for function parameters
- Use `Arc<str>` instead of `String` for shared immutable strings
- Return `Result<T, Error>` for all fallible operations
- Use `thiserror` for error types, not manual implementations
- Prefer iterators over manual loops
- Use `tokio` for all async operations
- Leverage RAII - resources should clean themselves up
- Version constraints should be parsed into structured types, not passed as strings internally

### Testing Requirements
- Unit tests for all complex logic (>20 lines or involves state)
- Integration tests for public APIs
- Use `proptest` for property-based testing where applicable
- Mock filesystem operations in tests
- Test both success and error paths
- Extensive tests for version constraint parsing and matching
- Test version resolution with complex dependency graphs

## Cross-Cutting Conventions

| Aspect | Decision | Justification |
|--------|----------|---------------|
| **Async runtime** | `tokio` everywhere | Shared reactor, zero thread explosion |
| **Database** | `sqlx` for SQLite | Async-first, compile-time checked queries |
| **HTTP client** | `reqwest` with tokio | Async HTTP with connection pooling |
| **Error model** | `thiserror` per crate + fine-grained types in `errors` crate | Type-safe error handling, Clone when possible |
| **Version specs** | Python-style constraints (`==`, `>=`, `~=`, etc.) | Flexible and familiar syntax for developers |
| **Version parsing** | `semver` crate with custom constraint parser | Battle-tested semver implementation |
| **Logging** | Events only - NO `info!`, `warn!`, `error!` | All output via event channel; JSON logs from subscriber |
| **Progress** | Broadcast `Event` enum via channels | Decouples core from UI details |
| **SBOM** | SPDX 3.0 JSON (primary), CycloneDX 1.6 (optional) | Built into every package via Syft |
| **Crypto** | Minisign signatures; BLAKE3 for hashing | Small trust root + fast hashing |
| **Unit tests** | Each crate self-contained; tempdir fixtures | No integration tests hit real `/opt/pm` |
| **Linting** | `#![deny(clippy::pedantic, unsafe_code)]` | Forces deliberate unsafe usage |
| **CI** | `cargo deny` plus `cargo audit` | Catches transitive vulnerabilities |

## Third-Party Dependencies

Key external crates used throughout the project:

| Crate | Version | License | Used by | Purpose |
|-------|---------|---------|---------|---------|
| `tokio` | `1.x` | MIT | All async crates | Async runtime |
| `sqlx` | `0.7` | MIT/Apache-2.0 | `state` | Async SQLite access |
| `reqwest` | `0.11` | MIT/Apache-2.0 | `net` | HTTP client |
| `dashmap` | `5.x` | MIT | `install` | Concurrent deduplication |
| `crossbeam` | `0.8` | MIT/Apache-2.0 | `install` | Lock-free queues |
| `num_cpus` | `1.x` | MIT/Apache-2.0 | `builder` | CPU count detection |
| `semver` | `1.x` | MIT/Apache-2.0 | `types` | Version parsing |
| `blake3` | `1.x` | CC0-1.0/Apache-2.0 | `hash` | Content hashing |
| `rhai` | `1.x` | MIT/Apache-2.0 | `package` | Recipe scripting |
| `thiserror` | `1.x` | MIT/Apache-2.0 | `errors` | Error derives |
| `serde` | `1.x` | MIT/Apache-2.0 | Most crates | Serialization |
| `tracing` | `0.1` | MIT | All crates (macros) / `sps2` (subscriber) | Structured logging |

**Note**: We avoid `sys-info` due to GPL-2.0 license. Load average detection uses `num_cpus` only.

## Project Structure

```
spsv2/:
Cargo.toml              # Workspace manifest
apps/
crates/
tests/                  # Integration tests

spsv2/apps/:
sps2/                   # CLI application

spsv2/apps/sps2/:
Cargo.toml
src/

spsv2/apps/sps2/src/:
main.rs

spsv2/crates/:
audit/                  # Future: CVE detection using SBOMs
builder/                # Package building with SBOM generation
config/                 # Configuration management
errors/                 # Fine-grained error types
events/                 # Event system for async communication
hash/                   # Content addressing (BLAKE3)
index/                  # Package registry/discovery
install/                # Installation operations
manifest/               # Repository manifest handling
net/                    # Network operations (downloads)
ops/                    # High-level operations orchestration
package/                # Package definition (Starlark) handling
resolver/               # Dependency resolution for install and build
root/                   # Filesystem root/prefix management
state/                  # State management and transitions
store/                  # Package store implementation
types/                  # Shared type definitions

spsv2/crates/audit/:
Cargo.toml
src/
tests/

spsv2/crates/audit/src/:
lib.rs

spsv2/crates/audit/tests/:
integration.rs

spsv2/crates/builder/:
Cargo.toml
src/
tests/

spsv2/crates/builder/src/:
lib.rs

spsv2/crates/builder/tests/:
integration.rs

spsv2/crates/config/:
Cargo.toml
src/
tests/

spsv2/crates/config/src/:
lib.rs

spsv2/crates/config/tests/:
integration.rs

spsv2/crates/errors/:
Cargo.toml
src/
tests/

spsv2/crates/errors/src/:
lib.rs

spsv2/crates/errors/tests/:
integration.rs

spsv2/crates/events/:
Cargo.toml
src/
tests/

spsv2/crates/events/src/:
lib.rs

spsv2/crates/events/tests/:
integration.rs

spsv2/crates/hash/:
Cargo.toml
src/
tests/

spsv2/crates/hash/src/:
lib.rs

spsv2/crates/hash/tests/:
integration.rs

spsv2/crates/index/:
Cargo.toml
src/
tests/

spsv2/crates/index/src/:
lib.rs

spsv2/crates/index/tests/:
integration.rs

spsv2/crates/install/:
Cargo.toml
src/
tests/

spsv2/crates/install/src/:
lib.rs

spsv2/crates/install/tests/:
integration.rs

spsv2/crates/manifest/:
Cargo.toml
src/
tests/

spsv2/crates/manifest/src/:
lib.rs

spsv2/crates/manifest/tests/:
integration.rs

spsv2/crates/net/:
Cargo.toml
src/
tests/

spsv2/crates/net/src/:
lib.rs

spsv2/crates/net/tests/:
integration.rs

spsv2/crates/ops/:
Cargo.toml
src/
tests/

spsv2/crates/ops/src/:
lib.rs

spsv2/crates/ops/tests/:
integration.rs

spsv2/crates/package/:
Cargo.toml
src/
tests/

spsv2/crates/package/src/:
lib.rs

spsv2/crates/package/tests/:
integration.rs

spsv2/crates/resolver/:
Cargo.toml
src/
tests/

spsv2/crates/resolver/src/:
lib.rs

spsv2/crates/resolver/tests/:
integration.rs

spsv2/crates/root/:
Cargo.toml
src/
tests/

spsv2/crates/root/src/:
lib.rs

spsv2/crates/root/tests/:
integration.rs

spsv2/crates/state/:
Cargo.toml
src/
tests/
migrations/

spsv2/crates/state/src/:
lib.rs

spsv2/crates/state/tests/:
integration.rs

spsv2/crates/state/migrations/:
0001_initial_schema.sql
0002_add_build_deps.sql

spsv2/crates/store/:
Cargo.toml
src/
tests/

spsv2/crates/store/src/:
lib.rs

spsv2/crates/store/tests/:
integration.rs

spsv2/crates/types/:
Cargo.toml
src/
tests/

spsv2/crates/types/src/:
lib.rs

spsv2/crates/types/tests/:
integration.rs

spsv2/tests/:
# Integration tests directory

Each crate follows standard Rust structure:
crate-name/:
Cargo.toml
src/
    lib.rs
    ...
tests/
    integration.rs
```

## Architecture Overview

### Crate Dependencies
- **Foundation layer**: `errors` and `types` depend on nothing except std/serde/thiserror
- **Base services**: `events`, `config`, `hash` depend only on foundation crates
- **Core services**: Can depend on foundation + base + other core services as needed
- **Higher services**: Can depend on any lower layer
- **Orchestration**: `ops` can depend on everything, but nothing depends on it
- Maintain acyclic dependencies - no circular imports
- Keep crates focused on single responsibilities

### Dependency Structure

**Foundation Layer (no dependencies):**
- `errors` - Error type definitions
- `types` - Core data structures, version parsing

**Base Services (depend on foundation):**
- `events` - Event definitions (depends on: errors, types)
- `config` - Configuration structures (depends on: errors, types)
- `hash` - BLAKE3 hashing (depends on: errors, types)

**Core Services:**
- `net` - Network operations (depends on: errors, types, events)
- `manifest` - Package manifests (depends on: errors, types, config, hash)
- `package` - Starlark package definitions (depends on: errors, types, hash)
- `root` - Filesystem operations (depends on: errors, types)
- `index` - Package index (depends on: errors, types, manifest)
- `store` - Content-addressed storage (depends on: errors, types, hash, root)

**Higher Services:**
- `resolver` - Dependency resolution (depends on: errors, types, index, manifest)
- `builder` - Package building (depends on: errors, types, events, package, manifest, hash, resolver)
- `state` - State management (depends on: errors, types, events, store, root)
- `audit` - Future CVE scanning (depends on: errors, types, manifest)

**Integration Layer:**
- `install` - Binary package installation (depends on: errors, types, events, net, resolver, state, store, audit + external: dashmap, crossbeam)

**Orchestration Layer:**
- `ops` - Command orchestration (depends on: all service crates)
- `sps2` - CLI application (depends on: ops, events)

**Key principles:**
- Acyclic dependencies - no circular imports allowed
- `ops` can depend on everything, but nothing depends on `ops`
- All crates can use `errors` and `types` as they're the foundation

**Example crate dependencies:**
- `install` needs: `state` (transitions), `store` (storage), `resolver` (deps), `net` (downloads)
- `builder` needs: `package` (Starlark), `manifest` (metadata), `hash` (checksums), `resolver` (build deps), SBOM generation
- `state` needs: `store` (linking), `root` (filesystem ops)
- `resolver` needs: `index` (available packages), `manifest` (dependencies)

**Version Resolution:**
The resolver uses Python-style version specifiers for maximum flexibility:
- Uses `semver` crate for version parsing and comparison
- Finds the highest version that satisfies all constraints
- Supports complex version ranges with multiple constraints
- Detects conflicts when constraints cannot be satisfied
- Handles both runtime dependencies (for install) and build dependencies (for build)
- Provides parallel execution plan for maximum performance

**Dependency resolution behavior:**
- **During `sps2 install`**: Downloads and installs binary packages with runtime dependencies
- **During `sps2 build`**: Resolves and installs build dependencies to temporary environment
- Build dependencies are installed to a temporary build environment
- Runtime dependencies of build deps are also installed (transitive)
- Circular dependencies are detected separately for runtime and build graphs
- Build environment is isolated from user's installed packages
- Only runtime dependencies are included in the final .sp package

### Dependency Resolution Architecture

The resolver provides deterministic, parallel package resolution for both installation and building:

#### Core Types
```rust
#[derive(Clone, Debug)]
pub enum DepKind { Build, Runtime }

#[derive(Clone, Debug)]
pub enum NodeAction {
    Download,  // Fetch binary .sp from repository
    Local,     // Use local .sp file
}

#[derive(Clone, Debug)]
pub struct DepEdge {
    pub name: String,
    pub spec: VersionSpec,      // e.g. ">=1.2.0,<2.0.0"
    pub kind: DepKind,
}

#[derive(Clone, Debug)]
pub struct ResolvedNode {
    pub name: String,
    pub version: Version,
    pub action: NodeAction,
    pub deps: Vec<DepEdge>,
    pub url: Option<String>,    // For downloads
    pub path: Option<PathBuf>,  // For local files
}
```

#### Resolution Algorithm
1. **Load manifests** for root packages from index or local .sp files
2. **Version selection**: For each package, find highest version matching all constraints
3. **Graph construction**: DFS traversal building full dependency graph
4. **Cycle detection**: Simple DFS with stack tracking
5. **Topological sort**: Kahn's algorithm produces installation order

#### Parallel Execution
```rust
// Concurrent download/install with dependency ordering
struct NodeMeta {
    action: NodeAction,
    in_degree: AtomicUsize,      // Remaining dependencies
    parents: Vec<PackageId>,     // For decrement notification
}

// Key data structures
let graph: HashMap<PackageId, Arc<NodeMeta>>;
let inflight: DashMap<PackageId, JoinHandle<Result<()>>>;  // Deduplication (dashmap crate)
let ready_queue: SegQueue<PackageId>;                      // Lock-free queue (crossbeam)
let semaphore: Arc<Semaphore>;                            // Concurrency limit (tokio)

// Execution flow
1. Push all nodes with in_degree=0 to ready_queue
2. Workers pull from queue, download/extract package
3. On completion, decrement parent in_degrees
4. Push newly-ready parents to queue
5. Continue until queue empty
```

**Dependencies**: The `install` crate needs `dashmap` for concurrent deduplication and `crossbeam` for lock-free queues.

#### Install vs Build Behavior

**During `sps2 install`**:
- Resolves runtime dependencies only
- Downloads binary packages in parallel
- Installs to main system state (`/opt/pm/live/`)
- No symlink management (single-prefix design)
- User must ensure `/opt/pm/live/bin` is in PATH

**During `sps2 build`**:
- Resolves build dependencies from recipe
- Downloads and installs build deps to `/opt/pm/build/<pkg>/deps/`
- Sets up isolated environment (PATH, PKG_CONFIG_PATH, etc.)
- Build deps are binary packages from repository
- After build completes, deps directory is cleaned up
- Only runtime deps are recorded in output .sp manifest

#### Performance Characteristics
- **Parallelism**: Limited by graph width (number of packages with no pending deps)
- **Deduplication**: Shared dependencies downloaded/installed only once
- **Early start**: Packages begin installing the moment their deps are ready
- **Network utilization**: Downloads overlap with extraction/linking
- **Typical speedup**: 3-5x over serial installation on fast networks

#### Example Resolution
```
Installing: jq (depends on oniguruma)
            curl (depends on openssl, zlib)

Execution order:
1. [Parallel] Download oniguruma, openssl, zlib
2. [Parallel] Install oniguruma, openssl, zlib
3. [Parallel] Download jq, curl
4. [Parallel] Install jq, curl
```

For builds, same logic applies but to temporary build environment.

### Error Handling Architecture

The `errors` crate provides fine-grained error types organized by domain:

```
crates/errors/:
src/:
src/lib.rs       # Re-exports all error types
src/network.rs   # NetworkError (connection, timeout, etc.)
src/storage.rs   # StorageError (disk full, permissions, etc.)
src/state.rs     # StateError (invalid transitions, conflicts)
src/package.rs   # PackageError (corrupt, missing deps, etc.)
src/...
```

Each error type:
- Implements `Clone` where possible (avoid storing non-clonable types)
- Uses `#[derive(thiserror::Error)]` for automatic Display/Error impl
- Provides context via `#[error("...")]` attributes
- Can be converted to a generic error for cross-crate boundaries

Example:
```rust
// In errors/src/network.rs
#[derive(Debug, Clone, thiserror::Error)]
pub enum NetworkError {
    #[error("connection timeout to {url}")]
    Timeout { url: String },

    #[error("download failed: {0}")]
    DownloadFailed(String),
}
```

### Type Definitions Architecture

The `types` crate provides core data structures including version specifications:

```rust
// types/src/version.rs
#[derive(Debug, Clone)]
pub enum VersionSpec {
    Exact(Version),           // ==1.2.3
    GreaterEqual(Version),    // >=1.2.0
    LessEqual(Version),       // <=2.0.0
    Greater(Version),         // >1.0.0
    Less(Version),            // <2.0.0
    Compatible(Version),      // ~=1.2.0
    NotEqual(Version),        // !=1.5.0
    And(Box<VersionSpec>, Box<VersionSpec>), // Multiple constraints
}

impl FromStr for VersionSpec {
    // Parse "package>=1.2.0,<2.0.0,!=1.5.0" into constraint tree
}

impl VersionSpec {
    pub fn matches(&self, version: &Version) -> bool { ... }
}
```

**Common types for events and operations:**
```rust
// types/src/lib.rs
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub version: Version,
    pub description: Option<String>,
    pub installed: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub name: String,
    pub version: Version,
    pub description: String,
    pub homepage: Option<String>,
}
```

### Event System and Async Architecture

#### Async Runtime
- Full async/await from the ground up
- Use `tokio` runtime with multi-threaded scheduler
- All I/O operations must be async (`tokio::fs`, `sqlx`, etc.)
- Use `spawn_blocking` sparingly for CPU-intensive work
- Channels for cross-crate communication (via `events` crate)

**Important**: Use modern async crates when tokio doesn't provide the functionality:
- **Database**: Use `sqlx` for SQLite operations (NOT `rusqlite` or blocking alternatives)
- **HTTP**: Use `reqwest` with tokio runtime (NOT blocking HTTP crates)
- **Process spawning**: Use `tokio::process` (NOT `std::process`)
- **File watching**: Use `notify` with tokio integration
- Only use sync/blocking crates when absolutely no async alternative exists

#### Event Communication
- **Use `tokio::sync::mpsc`** for all async channels
- Prefer `UnboundedSender/UnboundedReceiver` for event passing
- The `events` crate should export type aliases:
  ```rust
  pub type EventSender = tokio::sync::mpsc::UnboundedSender<Event>;
  pub type EventReceiver = tokio::sync::mpsc::UnboundedReceiver<Event>;
  ```
- All crates take an `EventSender` in their public APIs
- Only the CLI should own the `EventReceiver`
- Use `tokio::select!` for handling multiple channels
- Consider `tokio::sync::broadcast` if multiple consumers needed
- **NO direct logging** - No `println!`, `eprintln!`, `info!`, `warn!`, `error!` outside CLI
- All output goes through events - the CLI decides how to display

**Core Event enum variants:**
```rust
use crate::types::{PackageInfo, SearchResult, Version};

#[derive(Debug, Clone)]
pub enum Event {
    // Download events
    DownloadStarted { url: String, size: Option<u64> },
    DownloadProgress { url: String, bytes_downloaded: u64, total_bytes: u64 },
    DownloadCompleted { url: String },

    // Build events
    BuildStepStarted { package: String, step: String },
    BuildStepOutput { package: String, line: String },
    BuildStepCompleted { package: String, step: String },

    // State management
    StateTransition { from: Uuid, to: Uuid, operation: String },
    StateRollback { from: Uuid, to: Uuid },

    // Package operations
    PackageInstalling { name: String, version: Version },
    PackageInstalled { name: String, version: Version },
    PackageRemoving { name: String, version: Version },
    PackageRemoved { name: String, version: Version },

    // Resolution
    ResolvingDependencies { package: String },
    DependencyResolved { package: String, version: Version },

    // Command completion
    ListComplete { packages: Vec<PackageInfo> },
    SearchComplete { results: Vec<SearchResult> },

    // Errors and warnings
    Warning { message: String, context: Option<String> },
    Error { message: String, details: Option<String> },

    // Debug logging (when --debug enabled)
    DebugLog { message: String, context: HashMap<String, String> },

    // General progress
    OperationStarted { operation: String },
    OperationCompleted { operation: String, success: bool },
}
```

**Crates that emit events** (take EventSender):
- `net` - Download progress, connection status
- `install` - Installation steps, file operations
- `state` - State transitions, rollback operations
- `builder` - Build progress, compilation status
- `resolver` - Dependency resolution progress
- `audit` - CVE scan results
- `ops` - High-level operation status

## Execution Flow

### Entry Point
The `sps2` CLI application is the sole entry point and manages all user interaction:
- Parses command-line arguments
- Initializes the tokio runtime
- Creates event channels for async communication
- Owns the `EventReceiver` and handles all display/output
- Delegates operations to the `ops` crate

### Command Flow Architecture

**Flow sequence:**
1. User invokes command
2. sps2 CLI parses arguments
3. CLI creates event channel
4. CLI calls ops crate with EventSender
5. ops executes or delegates to specialized crates
6. Crates emit events through EventSender
7. CLI receives events via EventReceiver
8. CLI displays output to user

**Communication pattern:**
- One-way event flow: crates → EventSender → EventReceiver → CLI
- No direct output from crates (no println/logging)
- All user feedback goes through event channel

### Operations Hierarchy

The `ops` crate serves as the orchestration layer with a key architectural distinction:
- **Small operations** (list, info, search, etc.): Implementation logic lives IN the `ops` crate
- **Large operations** (install, build, etc.): `ops` just delegates to specialized crates

This keeps complex workflows isolated in their dedicated crates while simple operations don't need entire crates.

#### Operations Context
```rust
pub struct OpsCtx<'a> {
    pub store: &'a Store,
    pub state: &'a StateManager,
    pub index: &'a Index,
    pub net:   &'a NetClient,
    pub resolver: &'a Resolver,
    pub builder: &'a Builder,
    pub tx: EventSender,
}
```

#### Command Implementations

**Important Architecture Rule**:
- **Small operations**: Logic lives in `ops` crate, which calls into service crates for specific functionality
- **Large operations**: `ops` merely delegates to specialized crates that contain the full implementation

| Command | Type | Implementation | Calls into crates |
|---------|------|----------------|-------------------|
| **`reposync`** | Small | Logic in `ops` | `net` (download), `index` (update) |
| **`list`** | Small | Logic in `ops` | `state` (query packages) |
| **`info`** | Small | Logic in `ops` | `index` (details), `state` (status) |
| **`search`** | Small | Logic in `ops` | `index` (search) |
| **`cleanup`** | Small | Logic in `ops` | `state` (find orphans), `store` (GC) |
| **`install`** | Large | Delegates to `install` crate | `resolver` (runtime deps), `net` (downloads) |
| **`update`** | Large | Delegates to `install` crate | `resolver` (constraints), `net` (downloads) |
| **`upgrade`** | Large | Delegates to `install` crate | `resolver` (latest versions), `net` (downloads) |
| **`uninstall`** | Large | Delegates to `install` crate | `state` (removes package and orphaned deps) |
| **`build`** | Large | Delegates to `builder` crate | `resolver` (build deps), `install` (dep setup) |
| **`rollback`** | Small | Logic in `ops` | `state` (restore previous state) |
| **`history`** | Small | Logic in `ops` | `state` (list all states) |
| **`check-health`** | Small | Logic in `ops` | `state` (verify integrity), `store` (check refs) |

**`check-health` command specification:**
- **Input**: No arguments required
- **Operation**: Verifies system integrity by checking:
  - Database consistency (all referenced packages exist in store)
  - Store integrity (all store entries have valid manifests)
  - State directory structure (no orphaned staging dirs)
  - Permissions on critical paths
- **Output**: Table showing component status, or JSON with --json flag
- **Exit code**: 0 if healthy, 1 if issues found

**Example of small operation (in `ops`):**
```rust
// ops/src/list.rs
use crate::types::PackageInfo;

pub async fn list(ctx: &OpsCtx) -> Result<Vec<PackageInfo>> {
    // Logic lives here in ops
    let installed = ctx.state.get_installed_packages().await?;
    let enriched = installed.into_iter()
        .map(|p| enrich_with_metadata(p, ctx))
        .collect::<Result<Vec<_>>>()?;
    ctx.tx.send(Event::ListComplete { packages: enriched.clone() })?;
    Ok(enriched)
}
```

**Example of large operation (in `ops`):**
```rust
// ops/src/install.rs
pub async fn install(ctx: &OpsCtx, package_specs: &[String]) -> Result<OpReport> {
    // Determine if specs are local files or package names
    let install_requests = package_specs.iter()
        .map(|s| {
            if s.ends_with(".sp") && std::path::Path::new(s).exists() {
                InstallRequest::LocalFile(s.to_string())
            } else {
                // Parse version constraints for remote packages
                let spec = PackageSpec::parse(s)?;
                InstallRequest::Remote(spec)
            }
        })
        .collect::<Result<Vec<_>>>()?;

    // Delegate to specialized crate (binary install only)
    crate::install::execute_install(
        install_requests,
        ctx.resolver,
        ctx.net,
        ctx.state,
        ctx.store,
        ctx.tx.clone()
    ).await
}
```

#### Update vs Upgrade Distinction
- **`update`**: Only bumps compatible versions (respects `~=` semantics)
  - Package with `foo~=1.2.0` can update to 1.2.9 but not 1.3.0
  - Package with `foo>=1.0,<2.0` stays within those bounds
- **`upgrade`**: Allows major version jumps (ignores upper bounds)
  - Package with `foo~=1.2.0` can upgrade to 2.0.0 or higher
  - Still respects explicit `!=` exclusions for known bad versions
- Both return an `OpReport` that can be rendered as table, JSON, or plain text

### Event Flow Pattern
1. User invokes command:
   - `sps2 install package` - Install from repository
   - `sps2 install "package>=1.2.0,<2.0.0"` - Install with version constraints
   - `sps2 install ./package-1.2.0-1.arm64.sp` - Install local .sp file
2. CLI creates event channel and passes `EventSender` to ops
3. `ops::install()` called with package spec and event sender
4. ops determines if local file or remote package
5. For remote: delegates to `install::install()` with parsed version constraints
6. For local: delegates to `install::install_local()` with file path
7. Each crate sends progress/status events:
   ```rust
   sender.send(Event::DownloadProgress {
       url: download_url,
       bytes_downloaded: 1024000,
       total_bytes: 5242880
   })?;
   sender.send(Event::StateTransition { from, to })?;
   ```
8. CLI receives events and updates display accordingly
9. Final success/error event sent back to CLI

**Note**: Install is for binary packages only. To build from source, use `sps2 build recipe.star` which produces a .sp file.

### CLI Display Responsibilities
- Progress bars for downloads
- Status messages for operations
- Error formatting with helpful context
- Confirmation prompts when needed
- NO direct println!/eprintln! outside of CLI
- Machine-readable output modes (--json flag)
- Parse and validate version constraints before passing to ops
- **PATH reminder**: Show hint to add `/opt/pm/live/bin` to PATH after first install

**CLI usage examples:**
- `sps2 install jq` - Install latest binary package from repository
- `sps2 install "jq==1.7"` - Install exact version from repository
- `sps2 install "jq>=1.6,<2.0"` - Install with constraints from repository
- `sps2 install ./jq-1.7-1.arm64.sp` - Install from local .sp file
- `sps2 build jq.star` - Build package from recipe (produces .sp file)
- `sps2 build --network jq.star` - Build with network access enabled
- `sps2 update` - Update all packages respecting constraints
- `sps2 upgrade jq` - Upgrade to latest, ignoring upper bounds
- `sps2 rollback` - Revert to previous state
- `sps2 rollback <state-id>` - Revert to specific state
- `sps2 history` - List all states with timestamps
- `sps2 check-health` - Verify system integrity

**Note**: Ensure `/opt/pm/live/bin` is in your PATH after installation.

**Global CLI flags:**
- `--json` - Output in JSON format (for all commands)
- `--debug` - Enable debug logging to `/opt/pm/logs/`
- `--color <auto|always|never>` - Override color output
- `--config <path>` - Use alternate config file

**Command-specific flags:**
- `sps2 build --network` - Allow network access during build
- `sps2 build --jobs <n>` - Override parallel build jobs (0=auto)
- `sps2 rollback` - Revert to previous state
- `sps2 rollback <state-id>` - Revert to specific state
- `sps2 history` - List all states with timestamps
- `sps2 check-health` - Verify system integrity

### Operation Lifecycle
1. **Validation Phase** - Check permissions, validate arguments
2. **Planning Phase** - Resolve dependencies, check conflicts
3. **Execution Phase** - Perform actual operations
4. **Commit Phase** - Atomic state transitions
5. **Cleanup Phase** - Remove temporary files, update caches

Each phase emits appropriate events for CLI feedback.

## Core Systems

### Configuration Management

#### Configuration File
- **Location**: `~/.config/spsv2/config.toml` (follows XDG Base Directory spec)
- **Format**: TOML for consistency with Rust ecosystem
- **Precedence**: CLI flags > Environment variables > Config file > Defaults
- **Defaults location**: Hard-coded in `config` crate via `impl Default`

**Default values (in code):**
```rust
impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig {
                default_output: OutputFormat::Tty,
                color: ColorChoice::Auto,
                parallel_downloads: 4,
            },
            build: BuildConfig {
                build_jobs: 0,  // 0 = num_cpus::get()
                network_access: false,
            },
            security: SecurityConfig {
                verify_signatures: true,
                allow_unsigned: false,
                index_max_age_days: 7,
            },
            state: StateConfig {
                retention_count: 10,  // Keep last 10 states
                retention_days: 30,   // Or 30 days, whichever is less
            },
        }
    }
}
```

**Example config.toml:**
```toml
[general]
default_output = "tty"  # Options: plain, tty, json
color = "auto"          # Options: always, auto, never
parallel_downloads = 4

[build]
build_jobs = 0          # 0 = auto-detect from CPU count
network_access = false  # Allow network during builds

[security]
verify_signatures = true
allow_unsigned = false
index_max_age_days = 7

[state]
retention_count = 10    # Keep last N states
retention_days = 30     # Keep states newer than N days

[paths]
# Override default paths if needed (usually not recommended)
# store_path = "/opt/pm/store"
# state_path = "/opt/pm/states"
```

#### Environment Variables
- `SPSV2_OUTPUT` - Override output format
- `SPSV2_COLOR` - Override color setting
- `SPSV2_DEBUG` - Enable debug logging

### Atomic Update System

#### Filesystem Layout
```
/opt/pm/:
store/:                 # (A) Package Store - content-addressed storage
store/<hash>/:          # Immutable package contents
store/<hash>/files/     # Actual files
store/<hash>/blobs/     # Binary artifacts
store/<hash>/manifest   # Package metadata

states/:                # State directories
states/<uuid>/:         # (B) Archived states (previous roots)
states/staging-<uuid>/: # (C) Staging state (APFS clone)

live/:                  # (D) Active prefix (current root)
live/bin/:              # All installed binaries (add to PATH)

state.sqlite            # (F) State database with WAL
state.sqlite-wal        # SQLite write-ahead log
state.sqlite-shm        # SQLite shared memory

logs/:                  # Debug logs (when --debug is used)
logs/spsv2-<timestamp>.jsonl  # Structured JSON logs
```

#### State Management Architecture

**Components:**
1. **Package Store (A)**
   - Content-addressed storage using BLAKE3 hashes
   - Immutable files - never modified after creation
   - Hard-linked into state directories
   - Garbage collected based on reference counting

2. **State Directories (B, C, D)**
   - Each state is a complete root filesystem
   - Contains hard links to package store
   - Archived states kept for rollback
   - Staging state is APFS clone of current state

3. **SQLite State Database (F)**
   - Path: `/opt/pm/state.sqlite` (not in user's $HOME)
   - WAL mode for consistency
   - Tracks package references
   - Stores active state pointer
   - Records state transition history
   - **Must use `sqlx` for all database operations** (async-first)
   - **Migrations**: Embedded using `sqlx migrate` with versioned SQL files
   - Schema version tracked in database for safe upgrades

4. **Repo Manifest Cache (G)**
   - Immutable binary blobs
   - Read-only lookups during resolution
   - Updated via reposync operation

#### Atomic Update Process

**Installation Flow:**
```rust
// 1. Create staging directory as APFS clone
let staging_id = Uuid::new_v4();
let staging_path = format!("/opt/pm/states/staging-{}", staging_id);
apfs_clonefile("/opt/pm/live", &staging_path)?;

// 2. Modify staging directory
// - Add new package hard links from store
// - Remove old package hard links
// - Update metadata files

// 3. Begin database transaction
let tx = sqlx::Transaction::begin(&state_db).await?;

// 4. Record new state in database
sqlx::query("INSERT INTO states (id, parent, timestamp) VALUES (?, ?, ?)")
    .bind(&staging_id)
    .bind(&current_state_id)
    .bind(&now)
    .execute(&mut tx).await?;

// 5. Update package references
sqlx::query("INSERT INTO package_refs (state_id, package_hash) VALUES (?, ?)")
    .bind(&staging_id)
    .bind(&package_hash)
    .execute(&mut tx).await?;

// 6. Atomic filesystem swap
rename_swap(&staging_path, "/opt/pm/live")?;

// 7. Update active state pointer
sqlx::query("UPDATE active_state SET id = ?")
    .bind(&staging_id)
    .execute(&mut tx).await?;

// 8. Commit transaction
tx.commit().await?;

// 9. Archive previous state
rename(&old_live_path, &format!("/opt/pm/states/{}", old_state_id))?;
```

**Rollback Process:**
```rust
// 1. Find target state
let target_state = sqlx::query_as::<_, (String,)>(
    "SELECT path FROM states WHERE id = ?"
)
.bind(&rollback_id)
.fetch_one(&state_db).await?;

// 2. Atomic swap
rename_swap(&target_state.0, "/opt/pm/live")?;

// 3. Update database
sqlx::query("UPDATE active_state SET id = ?")
    .bind(&rollback_id)
    .execute(&state_db).await?;
```

#### Key Safety Properties
1. **Atomicity**: All updates use `renameat2` with `RENAME_SWAP` flag
2. **Consistency**: WAL-mode SQLite ensures database consistency
3. **Isolation**: Staging directory invisible until swap
4. **Durability**: Previous states preserved for rollback

#### APFS-Specific Optimizations
- Use `clonefile()` for instant, space-efficient copies
- Hard links for deduplication within states
- Set compression flags on `/opt/pm/store/`
- Leverage APFS snapshots for system-wide backups

#### Garbage Collection
- Reference counting in SQLite for store objects
- Configurable retention policy for old states
- **Default retention**: Keep last 10 states AND states from last 30 days (whichever is more)
- Never delete currently referenced packages
- Clean orphaned staging directories on startup
- **GC schedule**:
  - Runs automatically after install/uninstall/rollback operations
  - Also runs on `sps2` startup if last GC was >7 days ago
  - Startup GC runs after state DB is opened but before any operation planning
  - Manual trigger via `sps2 cleanup`
  - No background daemon - GC is always user-initiated or operation-triggered

### Build System

#### Build Architecture

**Build pipeline flow:**
1. `sps2 build recipe.star` command invoked
2. Sandboxed Starlark VM loads and validates recipe
3. Recipe calls Builder API methods (fetch, autotools, etc.)
4. Builder crate executes build in isolated environment
5. Package created and saved as .sp file:
   - Deterministic tar archive
   - manifest.toml with metadata
   - SBOM files (SPDX + optional CycloneDX)
   - Minisign signature
6. Output: `<name>-<version>-<revision>.<arch>.sp` file in current directory
7. User can then `sps2 install ./package.sp` to install locally

**Important**: `sps2 build` only produces packages, it does NOT install them. This follows Unix package manager conventions where building and installing are separate operations.

**Sandboxing:** Starlark VM provides limited API - no filesystem access, no network except fetch(), no exec()

#### Starlark Recipe Format
Build recipes are written in Starlark (Python-like) with a sandboxed, deterministic API:

```python
# nodejs.star - Example showing recipe structure
def metadata():
    """Return package metadata as a dictionary."""
    return {
        "name": "nodejs",
        "version": "20.11.0",
        "description": "JavaScript runtime",
        "homepage": "https://nodejs.org",
        "license": "MIT",
        # Dependencies will be added when builder is complete
        # "depends": ["libc++~=16.0.0"],
        # "build_depends": ["python3>=3.8"]
    }

def build(ctx):
    """Build the package using the provided context.
    
    Args:
        ctx: Build context with attributes:
            - ctx.NAME: package name from metadata
            - ctx.VERSION: package version from metadata
            - ctx.PREFIX: installation prefix (e.g. /opt/pm/live)
            - ctx.JOBS: number of parallel build jobs
    """
    # TODO: Once builder implementation is complete:
    # ctx.fetch(
    #     "https://nodejs.org/dist/v20.11.0/node-v20.11.0.tar.gz",
    #     "abc123...sha256..."
    # )
    # ctx.configure(["--prefix=" + ctx.PREFIX])
    # ctx.make(["-j" + str(ctx.JOBS)])
    # ctx.install()
    pass
```

**Version specifiers (Python-style):**
- `==1.2.3` - Exact version match
- `>=1.2.0` - Minimum version (inclusive)
- `<=2.0.0` - Maximum version (inclusive)
- `>1.0.0` - Greater than (exclusive)
- `<2.0.0` - Less than (exclusive)
- `~=1.2.0` - Compatible release (>=1.2.0, <1.3.0)
- `~=1.2` - Compatible release (>=1.2.0, <2.0.0)
- `!=1.5.0` - Exclude specific version
- `>=1.2,<2.0,!=1.5.0` - Multiple constraints (comma-separated)

**Compatible release (`~=`) explanation:**
- `~=1.2.3` means `>=1.2.3, <1.3.0` (patch updates only)
- `~=1.2` means `>=1.2.0, <2.0.0` (minor updates allowed)
- `~=1` means `>=1.0.0, <2.0.0` (major version pinned)

**Dependency handling:**
- `depends_on()`: Runtime dependencies that must be installed with the package
- `build_depends_on()`: Build-time only dependencies, available in build environment
- Build deps are automatically set up in PATH/PKG_CONFIG_PATH during build
- Only runtime deps are recorded in the final package manifest
- Build deps are never installed on end-user systems
- Dependencies are specified as strings with optional version constraints

**Sandboxing controls:**
- Max operations: 50,000,000 (prevent infinite loops)
- Max memory: 64 MiB
- No filesystem access except through Builder API
- No network access except through fetch()
- No environment variables or exec()

#### Builder API (Fluent Interface)

| Method | Effect | Notes |
|--------|--------|-------|
| `fetch(url, sha256)` | Downloads & verifies source | Retries + mirror fallback |
| `apply_patch(path)` | Applies a patchfile | Uses `patch -p1` |
| `autotools(args[])` | `./configure && make && make install` | Configures with PREFIX |
| `cmake(args[])` | CMake build helper | Sets CMAKE_PREFIX_PATH |
| `meson(args[])` | Meson build helper | |
| `cargo(args[])` | Cargo build helper | |
| `install()` | Finalizes staging & creates package | Called once at end |

**Build environment setup:**
- Build dependencies are automatically installed before `build()` runs
- Build deps are downloaded as binary packages from the repository
- `PATH` includes all build deps' bin directories
- `PKG_CONFIG_PATH` set up for all build deps
- `CFLAGS`/`LDFLAGS` configured for proper linking
- Build deps are NOT included in final package

#### Build Isolation
- Build prefix: `/opt/pm/build/<pkg>/<ver>/`
- Build deps prefix: `/opt/pm/build/<pkg>/<ver>/deps/`
- Staging directory: `/opt/pm/build/<pkg>/<ver>/stage/`
- Final installation: Content-addressed in `/opt/pm/store/<hash>/`
- Build dependencies installed to isolated `deps/` directory
- Environment variables set to use build deps (PATH, PKG_CONFIG_PATH, etc.)
- **Sandbox model**: $PREFIX isolation only (no chroot/container)
- **Network policy**: Disabled by default during builds (configurable via config.toml)
- **Build environment**:
  - Clean environment variables (minimal passthrough)
  - No access to user's home directory
  - Restricted to build prefix only
  - Network blocked unless explicitly enabled in recipe
- Relocatability scan to detect any hardcoded absolute paths
- Build failures if absolute paths found (ensures portability)
- Build deps are cleared after successful build (not included in package)

#### Integration with Atomic Updates
1. **Build Phase**: Package built in isolated `/opt/pm/build/` prefix with build deps, produces .sp file
2. **Distribution**: .sp file uploaded to CDN/GitHub Releases
3. **Install Phase**: User downloads .sp file (or provides local path)
4. **Store Phase**: Package contents extracted to content-addressed store
5. **Link Phase**: Store contents hard-linked into state directories with runtime deps
6. **Activation**: Atomic rename makes new state live

**Key point**: Building and installing are completely separate operations. Users typically only install pre-built binary packages. Building from source is only needed for package maintainers or custom packages.

### Package Format

#### .sp File Structure

| Component | Format | Purpose |
|-----------|--------|---------|
| **Payload** | `tar --deterministic \| zstd -19` | Reproducible compression |
| **manifest.toml** | TOML in archive root | Name, version, deps, hashes |
| **sbom.spdx.json** | SPDX 3.0 JSON | Primary SBOM format |
| **sbom.cdx.json** | CycloneDX 1.6 JSON (optional) | Secondary SBOM for compatibility |
| **Signature** | Detached `.minisig` | Minisign signature over all files |
| **Filename** | `<n>-<ver>-<rev>.<arch>.sp` | Unique identification |

**manifest.toml structure:**
```toml
[package]
name = "jq"
version = "1.7"
revision = 1
arch = "arm64"

[dependencies]
# Runtime dependencies - required to run the package
runtime = [
    "oniguruma==6.9.8",
    "libc++~=16.0.0"
]
# Build dependencies - only needed during compilation
build = [
    "autoconf>=2.71",
    "automake~=1.16.0",
    "libtool==2.4.7",
    "pkg-config>=0.29.2"
]

[sbom]
spdx = "sha256:4fa5..."
cyclonedx = "sha256:31d2..."  # optional
```

#### SBOM Generation (Built from Day 1)
- **Generator**: Syft ≥ 1.4 (deterministic, supports both formats)
- **When**: After `install()` completes, before packaging
- **Verification**: Re-run to ensure deterministic output
- **Coverage**: All files in staging directory
- **Exclusions**: Debug symbols (`*.dSYM`), configurable per recipe
- **Dependency tracking**: SBOMs include both runtime and build dependencies with clear labeling

**Builder API addition:**
```rust
b.auto_sbom(true)  // Enable SBOM generation (default: true)
b.sbom_excludes(["*.pdb", "*.dSYM", "*.a", "*.la"])  // Exclude patterns (static libs added)
```

#### Repository Index Format
```json
{
  "version": 1,
  "minimum_client": "0.1.0",
  "timestamp": "2025-05-29T12:00:00Z",
  "packages": {
    "jq": {
      "versions": {
        "1.7": {
          "revision": 1,
          "arch": "arm64",
          "sha256": "...",
          "download_url": "https://...",
          "minisig_url": "https://...",
          "dependencies": {
            "runtime": ["oniguruma==6.9.8", "libc++~=16.0.0"],
            "build": ["autoconf>=2.71", "automake~=1.16.0"]
          },
          "sbom": {
            "spdx": {
              "url": "https://.../jq-1.7-1.arm64.sbom.spdx.json",
              "sha256": "4fa5..."
            },
            "cyclonedx": {
              "url": "https://.../jq-1.7-1.arm64.sbom.cdx.json",
              "sha256": "31d2..."
            }
          }
        }
      }
    }
  }
}
```

**Index version policy:**
- If `index.version > client_supported_version`: Hard fail with clear error message
- Users must upgrade spsv2 to use newer index formats
- If `client_version < minimum_client`: Warn but continue (soft deprecation)
- Cache last known good index locally for offline use

**Dependency types:**
- **runtime**: Required for the package to function after installation
- **build**: Only needed during package compilation (not installed with package)
- Build deps are automatically available in build environment but not linked to final package
- Both runtime and build deps are satisfied by binary packages from the repository

### Security Model
- **Minisign** for package signatures (small attack surface)
- **SHA256** for content verification
- **SBOM** for supply chain transparency
- **Codesigning** for macOS Gatekeeper
- **Deterministic builds** for reproducibility

#### Key Distribution & Trust Root
- **Bootstrap key**: Embedded in CLI binary at compile time
- **Key storage**: `/opt/pm/keys/` directory with trusted public keys
- **Key format**: Minisign public key files (`.pub`)
- **Rotation process**:
  1. New key signed by old key (creates trust chain)
  2. Rotation announcement published as `keys.json` at repository root
  3. Both keys valid during transition period (30 days default)
  4. Old key expires after grace period
- **Rotation file location**: `https://cdn.sps.io/keys.json` (repository root, next to `index.json`)
- **Rotation file format** (`keys.json`):
  ```json
  {
    "current": {
      "id": "RWRzQJ6...",
      "pubkey": "untrusted comment: ...\nRWRzQJ6...",
      "valid_from": "2025-01-01T00:00:00Z"
    },
    "rotations": [{
      "new_key": "RWRnew...",
      "signature": "minisign signature of new key by old key",
      "valid_from": "2025-06-01T00:00:00Z",
      "old_key_expires": "2025-07-01T00:00:00Z"
    }]
  }
  ```
- **Index protection**:
  - `index.json` includes timestamp and is signed
  - Signature stored as adjacent `index.json.minisig` file
  - Clients reject indices older than 7 days (configurable)
  - Prevents CDN from serving stale but valid indices
- **Mirror verification**: All downloads verified against hashes in signed index

### CI/CD Pipeline

| Step | Implementation | Purpose |
|------|----------------|---------|
| Source cache | GitHub Actions cache by URL+SHA | Avoid re-downloading |
| Build matrix | `arch=[arm64]` `macos=[14]` | Platform coverage (15 when available) |
| Codesigning | `codesign --options=runtime --entitlements entitlements.plist` | Hardened runtime for notarization |
| Upload | GitHub Releases + CDN | Redundant distribution |
| Index | Static `index.json` with ETag | Efficient updates |
| MSRV check | See below | Ensure minimum Rust version |
| Warnings | `cargo clippy -- -D warnings` | Enforce zero warnings |

**MSRV Enforcement CI Job:**
```yaml
msrv:
  runs-on: macos-14
  steps:
    - uses: actions/checkout@v4
    - name: Install minimum supported Rust version
      run: |
        rustup toolchain install 1.86.0 --profile minimal
        rustup override set 1.86.0
    - name: Check with MSRV
      run: cargo +1.86.0 check --workspace --all-features
    - name: Test with MSRV
      run: cargo +1.86.0 test --workspace
```

**Code-signing entitlements (`entitlements.plist`):**
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.cs.allow-unsigned-executable-memory</key>
    <true/>
    <key>com.apple.security.cs.disable-library-validation</key>
    <true/>
</dict>
</plist>
```

**Entitlements justification:**
- `allow-unsigned-executable-memory`: Future-proofing for WASM/JIT plugins (Starlark uses bytecode interpreter)
- `disable-library-validation`: Needed to load packages that contain dylibs from `/opt/pm/live/lib`
- These are standard for package managers and development tools
- Alternative would break core functionality (no future JIT support, no dynamic libraries)

### Package Repository Strategy
- Start with essential developer tools (git, curl, openssl, etc.)
- Use AI-assisted recipe generation from source URLs
- Recipes must specify both runtime and build dependencies with appropriate version constraints
- Prefer `~=` for compatible releases, `>=` for minimum versions
- Use exact versions (`==`) only when compatibility requires it
- CI/CD builds all packages from recipes to produce binary .sp files
- Binary packages (.sp files) hosted on GitHub Releases + CDN
- Users install pre-built binary packages only
- No compilation happens on end-user systems
- No external package manager dependencies

## Release & Distribution

### Versioning Strategy
- **CLI version**: Semantic versioning (e.g., 0.1.0, 0.2.0, 1.0.0)
- **Index format version**: Integer increment (currently: 1)
- **Compatibility**: CLI checks index version and minimum_client field
- **Release channels**:
  - `stable`: Production-ready releases
  - `testing`: Pre-release testing (opt-in via config)

### Bootstrap Installation
```bash
#!/bin/bash
# Bootstrap installer for spsv2
SPSV2_VERSION="0.1.0"
SPSV2_URL="https://github.com/org/spsv2/releases/download/v${SPSV2_VERSION}/spsv2-darwin-arm64"
SPSV2_MINISIG="https://github.com/org/spsv2/releases/download/v${SPSV2_VERSION}/spsv2-darwin-arm64.minisig"

# Download and verify
curl -L -o /tmp/spsv2 "$SPSV2_URL"
curl -L -o /tmp/spsv2.minisig "$SPSV2_MINISIG"

# Embedded public key for bootstrap trust
PUBKEY="RWRzQJ6...bootstrap-key..."
echo "$PUBKEY" | minisign -V -p /dev/stdin -m /tmp/spsv2

# Install
sudo mkdir -p /opt/pm/live/bin
sudo mv /tmp/spsv2 /opt/pm/live/bin/
sudo chmod +x /opt/pm/live/bin/spsv2

# Setup PATH
echo 'export PATH="/opt/pm/live/bin:$PATH"' >> ~/.zshrc
echo "spsv2 installed! Restart your shell or run: export PATH=\"/opt/pm/live/bin:\$PATH\""
```

### PATH Policy
- **No symlinks**: We don't create any symlinks in `/usr/local/bin` or elsewhere
- **Single prefix**: All binaries live in `/opt/pm/live/bin/`
- **User responsibility**: Users must add `/opt/pm/live/bin` to their PATH
- **Shell integration**: Bootstrap script adds PATH export to shell rc file
- **Documentation**: README prominently shows PATH setup instructions

### Update Mechanism
- `sps2 self-update`: Updates spsv2 itself
- Downloads new version to temporary location
- Verifies signature before replacing
- Atomic replacement of binary
- Preserves configuration and state

## Implementation Guidelines

### Testing Strategy

#### Unit Testing
- Mock implementations for `Store`, `StateManager`, `Index`, etc.
- Use temp directories for filesystem operations
- Inject mocks into `OpsCtx` for isolated testing
- No network or APFS access in unit tests

#### Integration Testing
- Test complete workflows with real filesystem operations
- Use isolated `/tmp` prefixes for safety
- Verify atomic operations work correctly
- Test rollback scenarios

#### Test Fixtures
- Include sample `.sp` files in `tests/fixtures/` for integration tests
- Minimal packages: `hello-1.0.0-1.arm64.sp` with simple binary
- Complex packages: With multiple dependencies for resolver tests
- Invalid packages: For error handling tests

#### Test Coverage Requirements
- Every public API must have tests
- Both success and error paths must be tested
- Edge cases (empty state, corrupted data, etc.)
- Concurrent operation handling

#### Example Test Pattern
```rust
#[tokio::test]
async fn update_skips_major_versions() {
    let temp = TempDir::new().unwrap();
    let mock_index = MockIndex::new()
        .with_package("foo", "1.2.3")
        .with_package("foo", "2.0.0");

    let ctx = OpsCtx {
        index: &mock_index,
        // ... other mocks
    };

    // Assuming foo is installed with constraint "~=1.2.0"
    let report = ctx.update(&["foo"]).await.unwrap();
    assert_eq!(report.updated.len(), 0); // No update because 2.0.0 exceeds ~= constraint
}
```

### Performance Considerations

#### Async I/O
- Use `tokio::fs` for all file operations
- Use `sqlx` for all database operations (no blocking DB calls)
- Use `reqwest` for HTTP requests with connection pooling
- Concurrent downloads with connection pooling
- Parallel hash verification during installs
- Batch database operations where possible

#### APFS Optimizations
- `clonefile()` for instant staging directory creation
- Hard links to avoid data duplication
- Compression flags on `/opt/pm/store/`
- Avoid unnecessary stat() calls

#### Caching Strategy
- Repository index cached with ETag validation
- Package store is the cache (content-addressed)
- Build artifacts cached by source hash
- Build dependencies cached and reused across builds
- Runtime dependencies cached in package store
- Starlark recipes parsed and cached

#### Concurrency Limits
- Download pool: 4 concurrent connections (configurable)
- Hash verification: num_cpus threads
- Build jobs: Algorithm below
- Database connections: SQLx pool with 5 max connections (1 writer, 4 readers)

**Build concurrency algorithm:**
```rust
fn calculate_build_jobs(config_value: usize) -> usize {
    if config_value > 0 {
        config_value  // User override
    } else {
        // Auto-detect based on CPU count
        let cpus = num_cpus::get();

        // Use 75% of CPUs for builds, minimum 1
        // This leaves headroom for system responsiveness
        (cpus * 3 / 4).max(1)
    }
}
```

**Event channel notes:**
- Using unbounded channels for simplicity
- In practice, memory usage limited by operation scope
- Long builds with verbose output may buffer significant events
- **BuildStepOutput truncation**: Lines longer than 4KB are truncated with "..." suffix
- **Build log overflow**: After 10MB of output per step, emit warning and drop subsequent lines
- Future optimization: Consider bounded channels with back-pressure if needed

## Future: CVE Audit System (Low Priority)

**Note**: This functionality will be implemented after the core package manager is complete and stable.

### Architecture Overview
The `audit` crate will provide offline CVE scanning using embedded SBOMs:

```
sps2 audit [--all|--package <name>] [--fail-on critical]
         │
         ├─> Load SBOM from installed packages
         ├─> Query local vulnerability database
         └─> Report findings (table/json)
```

### Vulnerability Database Design
- **Format**: SQLite databases for offline queries (accessed via `sqlx`)
- **Sources**: NVD, OSV, GitHub Security Advisories
- **Updates**: Daily sync via `sps2 vulndb update`
- **Storage**: `/opt/pm/vulndb/` with versioned schemas

### Audit Workflow
1. Parse SBOM (SPDX/CycloneDX) from installed packages
2. Extract component identifiers (PURL, CPE)
3. Query local SQLite databases for matches
4. Filter by severity thresholds
5. Present results with remediation advice

### Implementation Plan (Future)
1. `vulndb` crate for database management
2. SBOM parser integration (reuse from builder)
3. CVE matching logic with semver awareness
4. CLI command (`sps2 audit`) with output formats
5. Post-install hooks for automatic scanning
6. Database update mechanism and CDN distribution

### Why This Design
- **Offline-first**: No privacy concerns from phoning home
- **Fast**: Local SQLite queries < 50ms per package
- **Accurate**: SBOM-based matching reduces false positives
- **Integrated**: Reuses existing SBOM infrastructure

This audit system will provide enterprise-grade supply chain security without compromising user privacy or adding network dependencies to the core package management operations.
