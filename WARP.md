# WARP.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

Project: sps2 — a modern, atomic package manager for macOS ARM64 with rollback and hermetic builds. Workspace is Rust-based with multiple apps and crates.

Sections
- Commands (build, lint, test, run, single test)
- Applications overview (sps2, sls, sbs)
- High-level architecture and data flows
- Notable docs and references

Commands
- Prereqs
  - Rust toolchain (MSRV: 1.90.0; workspace sets rust-version in Cargo.toml)
  - macOS Apple Silicon; many operations target aarch64-apple-darwin
  - Optional helper: just (recommended)

- Quickstart (use just, falls back to cargo if just is unavailable)
  - Build (release, target aarch64-apple-darwin):
    - just build
    - cargo build --release --target=aarch64-apple-darwin
  - Lint (Clippy across targets/features):
    - just lint
    - cargo clippy --all-targets --all-features
  - Format:
    - just fmt
    - cargo fmt
  - Test (entire workspace):
    - just test
    - cargo test
  - Run a specific app:
    - sps2: cargo run -p sps2 -- <args>
    - sls: cargo run -p sls -- <args>
    - sbs: cargo run -p sbs -- <args>

- Single test examples
  - By name pattern (fast path):
    - cargo test -p <crate> <substring>
  - Exact test in module, show output:
    - cargo test -p <crate> <module_or_test_path>::<test_name> -- --exact --nocapture

Applications
- sps2 (primary CLI)
  - Path: apps/sps2
  - Purpose: orchestrates package lifecycle commands (install, update/upgrade, uninstall, build/pack, reposync, history/rollback, verify/guard, audit/vulndb, keys/repo management). Provides JSON or table output and structured progress events.
  - Global flags (from apps/sps2/src/cli.rs):
    - --json (machine-readable output)
    - --debug (enable debug logging to /opt/pm/logs)
    - --color {auto,always,never}
    - --config <PATH>, --builder-config <PATH>
    - --check (dry-run style preview where supported)
  - Representative commands (see README for usage examples): install, update, upgrade, uninstall, build, pack, list, info, search, reposync, cleanup, rollback, history [--all --verify --limit], check-health, vulndb {update|stats}, audit [--package --severity --fail-on-critical], self-update, draft, verify [--heal --level --scope --sync-refcounts].

- sls (Store List)
  - Path: apps/sls
  - Purpose: ls-like explorer for the content-addressed store (/opt/pm/store). Maps file hashes to real file paths and package origins using the state DB.
  - Key flags: --long/-l, --hash (suppress filename mapping), --recursive/-R, --store <DIR>, --db <PATH>, --packages/-p (list packages view), --no-color.

- sbs (SPS2 build/publish tooling)
  - Path: apps/sbs
  - Purpose: local repository tooling to publish .sp artifacts, rebuild/sign indices, and initialize repos/keys.
  - Subcommands:
    - publish --repo-dir <DIR> --base-url <URL> --key <PATH> [--pass <...>] <package.sp>
    - update-indices --repo-dir <DIR> --base-url <URL> --key <PATH> [--pass <...>]
    - repo-init --repo-dir <DIR> [--pubkey <PUBFILE> | --generate --out-secret <PATH> --out-public <PATH>] [--comment <TEXT>]

High-level architecture and data flows
- Workspace layout (big picture, not exhaustive)
  - apps/
    - sps2: CLI frontend, event loop, rendering (apps/sps2/src/*.rs)
    - sls: CAS explorer
    - sbs: repo publishing and index signing
  - crates/ (core subsystems)
    - ops: orchestration layer called by sps2; implements small ops directly, delegates heavy ops (install/build/pack) to specialized crates. Builds a typed OpsCtx that wires store/state/index/net/resolver/builder/events/config.
    - builder: builds packages from YAML recipes into .sp artifacts; provides build plans, isolated environments, QA pipeline, SBOM and manifest generation, and signing.
    - install: high-performance installation engine with parallel download/prepare pipeline and AtomicInstaller for two-phase commit to the live state.
    - store: content-addressed storage (CAS) for packages; extracts, hashes, deduplicates files; persists manifest.toml, files.json, SBOMs per package hash; exposes link/verify/GC primitives.
    - state: SQLite-backed system state (installed packages, files, refcounts, states, history). Supports migrations, live slot management, and queries.
    - resolver: dependency resolver (SAT-backed) producing an execution plan for install/update/upgrade.
    - net: HTTP client with timeouts/retries/UA policy used across fetch operations.
    - guard: verification/healing across live and store, with discrepancy reporting and optional refcount sync.
    - events: domain event model for download/build/install/guard/etc., progress trackers, and structured logging.
    - config: layered configuration (file, env, CLI) plus fixed_paths for system directories (/opt/pm/...).
    - platform: macOS platform abstraction (filesystem operations, APFS compression, process helpers).
    - index/repository/signing/types/errors/hash/resources/...: supporting crates for repository index management, signing, shared types, error taxonomy, hashing (BLAKE3/xxh3), and resource management.

- Initialization and configuration (apps/sps2/src/main.rs and setup.rs)
  - Config precedence: file (or defaults) → environment → CLI flags.
  - SystemSetup ensures system directories exist, seeds default keys, checks permissions, and initializes components in order: store → state → index (cached or empty) → net → resolver → builder → platform cache → startup maintenance (GC of old states and orphaned staging).
  - Fixed paths (via sps2_config::fixed_paths):
    - /opt/pm/live (active state; add /opt/pm/live/bin to PATH to use installed binaries)
    - /opt/pm/store (CAS)
    - /opt/pm/states (historical states for rollback)
    - /opt/pm/state.sqlite (database)
    - /opt/pm/logs (debug logs)

- Command execution flow (install as example)
  1) sps2 parses CLI and builds OpsCtx with event channel.
  2) ops::install determines local vs remote mix and pushes a correlation id for tracing.
  3) For remote packages, resolver::resolve_with_sat produces an execution plan (download vs reuse from store cache).
  4) ParallelExecutor performs concurrent fetch/store/prepare with progress events; prepared packages are handed to AtomicInstaller.
  5) Atomic installation updates the live state via a two-phase commit, yielding a new state_id and enabling instant rollback.
  6) OutputRenderer prints a table or JSON; PATH reminder is shown on install.

- Build/Pack flow
  - builder parses YAML recipes into a BuildPlan; executes isolated build steps; runs post-processing and quality pipeline; generates SBOM and manifest; signs and emits a deterministic .sp.
  - pack mirrors build’s post/QA/signing paths but can also package directly from a staging directory with a manifest (advanced users). pack --no-post skips post and QA.

- Verification and healing
  - guard verifies live/store based on level (quick/standard/full) and scope (live/store/all). With --heal, attempts automatic remediation and optionally syncs DB refcounts from the active state.

- Events and observability
  - The CLI consumes events concurrently with command execution (tokio::select loop), feeding both structured logging (tracing) and user-friendly messages/tables. Global --json toggles machine-readable output.

Notable docs and references
- README.md includes: early-development status; features; install/draft/build examples; CAS directory structure; developer tools (sls examples).
- BUILD_SCRIPT_DOCUMENTATION.md: YAML recipe format reference for builder/pack.
- CONTRIBUTING.md and CODE_OF_CONDUCT.md apply for contributions.

Tips specific to this repo (non-generic, derived from code/docs)
- macOS Apple Silicon is the target; release builds default to aarch64-apple-darwin via just build. Use cargo build --release --target=aarch64-apple-darwin for parity without just.
- To explore the CAS and package/file mappings, prefer sls over manual filesystem traversal (supports colored, recursive, packages view, and mapping via the DB).
- For machine integrations, prefer sps2 --json and parse the structured OperationResult (see apps/sps2/src/display.rs for shapes or use the enum tags from ops::OperationResult).
