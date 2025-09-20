# sps2 Events & Errors Deep Review

## 1. Executive Summary
**Top issues (severity ordered):**
1. Massive unused surface: 480 event variants exist but only 158 are referenced; 93/229 error variants never appear at call-sites. This bloat obscures the active contract and confuses contributors.
2. Progress duplication: 45 domain variants embed progress metrics that belong in `ProgressEvent`, while the CLI never consumes the advanced progress channel. Progress data is fragmented and effectively lost.
3. Error leakage into events: nearly every domain enum has a `*Failed { error: String }` payload, duplicating `sps2_errors` but throwing away structured detail (retryable, source, remediation).
4. Missing correlation metadata: there is no shared `EventMeta` (ids, severity, source). Emitters invent ad-hoc IDs, preventing UI/log correlation and telemetry aggregation.
5. Payload inflation: large `Vec`, `PathBuf`, and `Duration` payloads are cloned and serialized without consumers, stressing the event bus and complicating future API stability.

**High-impact wins (1–2 week effort):**
- Prune unused event variants/enums (start with `AcquisitionEvent`, `PythonEvent`, unused install phases).
- Remove `error: String` fields in events; rely on structured `Error` propagation and log once in CLI.
- Introduce a thin `ProgressUpdate` API and migrate current emitters to it; stop emitting domain `*Progress` variants.
- Add `EventMeta { id, parent, issued_at, level, source, correlation }` and emit it from `OpsCtx`.

**30 / 60 / 90-day refactor roadmap:**
- **30 days:** inventory cleanup + guardrails. Delete unused variants, wire `EventMeta`, ensure CLI handles `ProgressEvent`. Ship lint to forbid new `error: String` payloads. Provide migration shims.
- **60 days:** land consolidated Domain/Progress/Diagnostic enums, migrate emitters incrementally (install/update/build/guard). Introduce durable `ErrorCode` & severity on `sps2_errors::Error`.
- **90 days:** complete consumer updates (CLI, logging, tests), remove deprecated variants, add snapshot/property tests for event streams, document contracts.

## 2. Event & Error Inventory
Artifacts generated in `refactor/`:
- `EVENTS_INVENTORY.json` — 60 type definitions extracted from `crates/events`.
- `event_variant_usage.json` — call-site map with 170 variant references.
- `ERRORS_INVENTORY.json` — 54 type definitions from `crates/errors`.
- `error_call_sites.json` — 154 distinct error references.

**Domain event density (top 12 by variants):**

| Event enum | Module | Variants | Used | Unused |
| --- | --- | ---: | ---: | ---: |
| ResolverEvent | events::resolver | 35 | 3 | 32 |
| AcquisitionEvent | events::acquisition | 32 | 2 | 30 |
| UninstallEvent | events::uninstall | 29 | 5 | 24 |
| UpdateEvent | events::update | 28 | 6 | 22 |
| BuildEvent | events::build | 27 | 13 | 14 |
| StateEvent | events::state | 25 | 17 | 8 |
| InstallEvent | events::install | 24 | 10 | 14 |
| PackageEvent | events::package | 23 | 16 | 7 |
| DownloadEvent | events::download | 22 | 9 | 13 |
| GuardEvent | events::guard | 19 | 11 | 8 |

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
- CLI handles only 4 `DownloadEvent` variants and 6 `InstallEvent` variants; the remainder never reach the UI (`apps/sps2/src/events.rs`, `logging.rs`).
- `ProgressEvent` is emitted 35 times but never matched by any consumer — progress updates do not surface.
- Errors most frequently constructed: `InstallError::InvalidPackageFile` (84 uses), `StorageError::IoError` (66), `PlatformError::FilesystemOperationFailed` (41).

## 3. Overlap & Redundancy Report
Full narrative in `refactor/overlap_report.md`. Key highlights:
- 45 domain variants replicate progress semantics; retain only high-level milestones in domain enums.
- 93 unused error variants increase maintenance cost with no benefit.
- Progress duplication + missing meta prevents correlating failures with progress stalls, despite emitting both signals.

## 4. Target Architecture (Proposed)
Goals: separate **Domain**, **Progress**, **Diagnostic** channels, enforce shared metadata, and attach structured errors. Foundational types now live in-code (`crates/events/src/meta.rs`, `crates/errors/src/structured.rs`); illustrative wiring remains in `refactor/proposed_events.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMeta {
    pub id: Uuid,
    pub parent: Option<Uuid>,
    pub issued_at: DateTime<Utc>,
    pub level: Level,
    pub source: &'static str,
    pub correlation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppEvent {
    Domain(EventMeta, DomainEvent),
    Progress(EventMeta, ProgressEvent),
    Diagnostic(EventMeta, DiagnosticEvent),
}
```

`refactor/proposed_errors.rs` outlines a companion error envelope with `ErrorCode`, `Severity`, and `ErrorContext`. Domain-specific errors collapse into reusable constructors supplying hints and retryability for the UI.

## 5. Concrete Refactor PR Plan
1. **Inventory pruning (PR #1)**
   - Delete unused event enums/variants confirmed by `event_variant_usage.json`.
   - Add `#[deprecated]` shims for any API exported publicly; guard with `cfg(test)` to keep fixtures compiling.
   - Introduce `#[deny(clippy::large_enum_variant)]` on `events` crate to flag oversized payloads.

2. **Event meta introduction (PR #2)**
   - Add `EventMeta` struct and change `EventEmitter::emit` signature to accept `(EventMeta, AppEventPayload)`.
   - Implement meta construction inside `OpsCtx` (auto-generate IDs, timestamps, correlation).
   - Update CLI consumers to read the meta (log level, source).

3. **Progress channel consolidation (PR #3)**
   - Introduce `ProgressUpdate` enum (Started/Advanced/Completed/Failed) with phase identifiers.
   - Replace domain `*Progress` variants with progress emissions; update CLI to render progress (TTY + JSON).
   - Deprecate builders like `ProgressEvent::started_with_phases` in favour of tracker APIs.

4. **Error envelope upgrade (PR #4)**
   - Implement `ErrorCode`, `Severity`, `ErrorContext`; add constructors for common operations.
   - Update major error emitters (install/update/build) to return the new enriched `Error`.
   - Wire CLI/logging to display `code`, `severity`, and `hint()`.

5. **Domain event slimming (PR #5)**
   - Reduce domain enums to milestone triads (`Started`, `Succeeded`, `Failed`) plus essential metadata.
   - Replace `error: String` with `ErrorCode` references; optionally embed `Option<ErrorContextRef>`.
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
  - Attach `ErrorCode` / `Severity` instead of raw strings; include hints via `Error::hint()`.
  - Event meta must include `source` (module path) and `correlation` (operation scope).
- **Versioning policy:**
  - Add new variants with `#[non_exhaustive]` enums; mark deprecated items with `since =` and remove after 2 minor releases.
  - Maintain a changelog documenting added/removed event variants and error codes.
- **Operational recipes:**
  - **Install:** `InstallStarted` → `Progress` phases (`Resolve`, `Fetch`, `Stage`, `Commit`) → `InstallCommitted` or `InstallRolledBack` with linked `ErrorCode`.
  - **Update:** `UpdateStarted` → per-package `Progress` children → `UpdateCompleted` or `UpdateFailed` with aggregated error summary.
  - **Build:** `BuildStarted` → `Progress` phases (`Fetch`, `Configure`, `Compile`, `Package`) → `BuildSucceeded` / `BuildFailed` + error context.

## 7. Validation & Tests
- **Snapshot tests:** capture event streams for `install`, `update`, `uninstall`, and failed `build` to guard regressions (TTY + JSON mode).
- **Property tests:** ensure progress streams obey invariants (`Started` → `...` → `Completed|Failed`, totals never decrease, `Completed` implies `current == total`).
- **Golden logs:** verify divergence between progress bar output and structured logs stays within budget (progress channel silent in JSON mode).
- **Compile-time checks:** feature-gated lint that forbids `error: String` fields and enforces `EventMeta` arguments (use a custom Clippy lint or `deny` macro).
- **CI gates:** run `cargo fmt`, `cargo clippy --all-targets --all-features`, `cargo test --workspace`, `cargo-udeps` to guarantee no resurrected dead code, and optional `cargo-semver-checks` for public crates.

---
**Next actions:** kick off PR #1 (variant pruning + progress handler wiring) and socialize the proposed contracts with CLI/platform teams before tackling meta introduction.
