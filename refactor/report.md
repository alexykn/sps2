# sps2 Events & Errors Deep Review

## 1. Executive Summary
**Top issues (severity ordered):**
1. Residual progress duplication: domain enums still emit progress-shaped variants even though the consolidated `ProgressEvent` channel now drives CLI rendering, keeping the surface larger than necessary.
2. Error leakage into events: `*Failed { error: String }` variants persist across domains, discarding structured context from `sps2_errors` and preventing users from seeing retryability or remediation hints.
3. Unused surface area: 49 event variants and 93 error variants have no call-sites post-trim, continuing to obscure the active contract for contributors.
4. Payload inflation: large `Vec`, `PathBuf`, and `Duration` payloads remain in events without consumers, complicating stabilization and adding serialization cost.
5. Domain error enums lack consistent `UserFacingError` implementations, so CLI/JSON output cannot yet surface hints uniformly without ad-hoc formatting.

**High-impact wins (1–2 week effort):**
- Prune or deprecate the lingering progress-shaped domain variants now that `ProgressEvent` updates are fully wired through the CLI and logging.
- Extend the lightweight `UserFacingError` trait to the remaining domain enums so callers can surface hints and retryability without heavy envelopes.
- Continue pruning/deprecating unused variants (focus on `PythonEvent` and legacy progress helpers) to keep the surface comprehensible.
- Add payload size guardrails (lint/tests) to prevent regressions as the event surface shrinks.

**30 / 60 / 90-day refactor roadmap:**
- **30 days:** retire duplicated domain progress variants, fill in `UserFacingError` gaps, and remove the most obvious unused enums (starting with `PythonEvent`).
- **60 days:** slim the remaining domain enums to milestone triads, replace stringly failure payloads with structured context, and update CLI consumers accordingly.
- **90 days:** remove deprecated variants, finalize diagnostics taxonomy, and backstop with snapshot/property tests plus doc updates for contributor guidelines.

## 2. Event & Error Inventory
Artifacts generated in `refactor/`:
- `EVENTS_INVENTORY.json` — 43 type definitions extracted from `crates/events` (post-prune).
- `event_variant_usage.json` — call-site map with 170 variant references.
- `ERRORS_INVENTORY.json` — 54 type definitions from `crates/errors`.
- `error_call_sites.json` — 154 distinct error references.

**Domain event density (top 12 by variants):**

| Event enum | Module | Variants | Used | Unused |
| --- | --- | ---: | ---: | ---: |
| AppEvent | events::mod | 17 | 17 | 0 |
| StateEvent | events::state | 17 | 17 | 0 |
| PackageEvent | events::package | 16 | 16 | 0 |
| ProgressEvent | events::progress | 14 | 12 | 2 |
| BuildEvent | events::build | 13 | 13 | 0 |
| PlatformEvent | events::platform | 12 | 12 | 0 |
| GuardEvent | events::guard | 11 | 11 | 0 |
| InstallEvent | events::install | 10 | 10 | 0 |
| PythonEvent | events::python | 10 | 0 | 10 |
| DownloadEvent | events::download | 9 | 9 | 0 |
| AuditEvent | events::audit | 8 | 8 | 0 |
| GeneralEvent | events::general | 8 | 8 | 0 |

Total variants across the event surface now sit at **207**, with **49** still unused (all concentrated in legacy progress helpers and the dormant `PythonEvent` domain).

**Error density (full list):**

| Error enum | Module | Variants | Used | Unused |
| --- | --- | ---: | ---: | ---: |
| BuildError | build | 44 | 32 | 12 |
| InstallError | install | 33 | 18 | 15 |
| OpsError | ops | 24 | 15 | 9 |
| Error (top-level) | lib | 16 | 2 | 14 |
| NetworkError | network | 15 | 10 | 5 |
| PackageError | package | 15 | 7 | 8 |
| AuditError | audit | 14 | 7 | 7 |
| PlatformError | platform | 12 | 10 | 2 |
| StorageError | storage | 12 | 8 | 4 |
| GuardError | guard | 10 | 10 | 0 |
| StateError | state | 9 | 3 | 6 |
| ConfigError | config | 8 | 7 | 1 |
| VersionError | version | 5 | 3 | 2 |

**Call-site insights:**
- Download/install/build/guard domains now map 1:1 between emitted variants and CLI consumers; only the dormant `PythonEvent` family remains unused.
- `ProgressEvent` is emitted 35 times and now feeds the CLI progress renderer introduced during Phase 3 consolidation.
- Errors most frequently constructed: `InstallError::InvalidPackageFile` (84 uses), `StorageError::IoError` (66), `PlatformError::FilesystemOperationFailed` (41).

## 3. Overlap & Redundancy Report
Full narrative in `refactor/overlap_report.md`. Key highlights:
- 45 domain variants replicate progress semantics; retain only high-level milestones in domain enums.
- 93 unused error variants increase maintenance cost with no benefit.
- Progress duplication + missing meta prevents correlating failures with progress stalls, despite emitting both signals.

## 4. Target Architecture (Proposed)
Goals: separate **Domain**, **Progress**, **Diagnostic** channels, enforce shared metadata, and keep error handling simple. Typed events (`crates/events/src/meta.rs`, `crates/events/src/lib.rs`) carry correlation and severity, while error display relies on the lightweight `UserFacingError` trait instead of envelope types. The runtime wire format uses the new `EventMessage` envelope:

```rust
#[derive(Clone, Debug)]
pub struct EventMessage {
    pub meta: EventMeta,
    pub event: AppEvent,
}

impl EventEmitter for OpsCtx {
    fn emit(&self, event: AppEvent) {
        let meta = self.meta_for(&event);
        self.emit_with_meta(meta, event);
    }
}
```

Domain events stay lightweight milestones, while the progress tracker now emits `ProgressManager`-generated `EventMessage`s with deterministic `parent_id` / `correlation_id`. The guard subsystem now owns its own discrepancy contexts (`crates/guard/src/diagnostics.rs`), keeping the shared errors crate enum-only.

## 5. Concrete Refactor PR Plan
1. **Inventory pruning (PR #1 — shipped)**
   - Unused variants culled or flagged; inventories kept in `refactor/` for follow-up removals.

2. **Event metadata pipeline (PR #2 — shipped)**
   - `EventMessage` envelope + `EventEmitter::emit_with_meta` provide IDs, levels, sources, and correlation IDs to every consumer (CLI + tracing already reads them).

3. **Progress channel consolidation (PR #3 — shipped)**
   - Streamlined progress tracker (`ProgressEvent` + `ProgressManager`) now emits through the shared envelope; CLI/JSON renderers consume it. Remaining work is to prune duplicate domain variants (tracked under PR #5).

4. **User-facing hints (PR #4 — next)**
   - Ensure all high-level operations implement the `UserFacingError` trait so CLI/logging/progress paths can display concise messages, hints, and retryability flags without additional envelopes.

5. **Domain event slimming (PR #5)**
   - Reduce domain enums to milestone triads (`Started`, `Succeeded`, `Failed`) plus essential metadata.
   - Replace `error: String` payloads with references to the underlying domain error (or remove them entirely) so `UserFacingError` can supply presentation details.
   - Update CLI pattern matches accordingly.

6. **Consumer hardening & telemetry (PR #6)**
   - Add snapshot tests for install/update/remove event streams.
   - Emit metrics on dropped events / backpressure; ensure CLI warns when not consuming progress.
   - Remove deprecated variants after 1 release (feature flag to fail CI when used).

## 6. Contracts & Guidelines
- **When to emit:**
  - `DomainEvent::{Started, Succeeded, Failed}` for lifecycle milestones visible to multiple consumers.
  - `ProgressEvent::{Started, Advanced, Completed, Failed}` for high-frequency updates; always include `meta.parent` referencing the domain event ID.
  - Return `Result<T, Error>` for control flow; emit `DomainEvent::Failed` only after returning the error.
- **Payload constraints:**
  - Prefer scalar IDs (`PkgId`, `Phase`) over large vectors; references ≥ 1KB require justification.
  - Avoid stringly error payloads; rely on domain errors plus `UserFacingError` for hints/retryability when a failure milestone must be emitted.
  - Event meta must include `source` (module path) and `correlation` (operation scope).
- **Versioning policy:**
  - Add new variants with `#[non_exhaustive]` enums; mark deprecated items with `since =` and remove after 2 minor releases.
  - Maintain a changelog documenting added/removed event variants and error codes.
- **Operational recipes:**
- **Install:** `InstallStarted` → `Progress` phases (`Resolve`, `Fetch`, `Stage`, `Commit`) → `InstallCompleted` or `InstallFailed` with `UserFacingError`-derived messaging.
- **Update:** `UpdateStarted` → per-package `Progress` children → `UpdateCompleted` or `UpdateFailed` with aggregated error messaging.
- **Build:** `BuildStarted` → `Progress` phases (`Fetch`, `Configure`, `Compile`, `Package`) → `BuildSucceeded` / `BuildFailed` + error context surfaced via `UserFacingError`.

## 7. Validation & Tests
- **Snapshot tests:** capture event streams for `install`, `update`, `uninstall`, and failed `build` to guard regressions (TTY + JSON mode).
- **Property tests:** ensure progress streams obey invariants (`Started` → `...` → `Completed|Failed`, totals never decrease, `Completed` implies `current == total`).
- **Golden logs:** verify divergence between progress bar output and structured logs stays within budget (progress channel silent in JSON mode).
- **Compile-time checks:** feature-gated lint that forbids `error: String` fields and enforces `EventMeta` arguments (use a custom Clippy lint or `deny` macro).
- **CI gates:** run `cargo fmt`, `cargo clippy --all-targets --all-features`, `cargo test --workspace`, `cargo-udeps` to guarantee no resurrected dead code, and optional `cargo-semver-checks` for public crates.

---
**Next actions:** complete PR #4 by extending `UserFacingError` coverage across the remaining domains, queue the domain-progress pruning work for PR #5, and regenerate the inventories once those variants are removed so downstream consumers stay in sync.
