# sps2 Events & Errors Deep Review

## 1. Executive Summary
**Top issues (severity ordered):**
1. Acquisition/resolver batches still ship verbose progress-style variants; now that download/install/uninstall/state emit milestone triads, those remaining domains stand out as the last large surfaces.
2. Several failure events (e.g., acquisition, guard, repo) continue to carry `error: String` payloads, preventing consumers from surfacing retryability hints supplied by `sps2_errors`.
3. The generated inventories (`EVENTS_INVENTORY.json`, `event_variant_usage.json`) lag behind the trimmed surface, so contributors still see Python/progress variants that no longer exist.
4. A handful of event payloads (resolver/acquisition) retain heavyweight vectors and `PathBuf`s even though the CLI/logging ignore them.
5. Progress/state contract tests are still missing; without snapshots/property checks it is easy to reintroduce noisy variants.

**High-impact wins (1–2 week effort):**
- Collapse acquisition/resolver batch events to the same `Started/Completed/Failed + ProgressEvent` contract now used for install/uninstall.
- Normalize failure payloads in guard/repo/acquisition to expose `retryable`/`UserFacingError` data instead of free-form strings.
- Regenerate the inventories in `refactor/` so the analysis artefacts match the slimmer surface (download/install/uninstall/state).
- Add payload size guardrails and event-stream snapshots for install/update/remove to lock in the triad pattern.

**30 / 60 / 90-day refactor roadmap:**
- **30 days:** finish pruning acquisition/resolver progress variants, refresh the inventories/docs, and land event-stream snapshots for install/update/remove.
- **60 days:** migrate remaining failure payloads to structured context, wire retryability into CLI/logging everywhere, and harden progress/drop metrics.
- **90 days:** remove any compatibility shims, finalize contributor guidance, and add automated checks (lint/tests) that prevent large payloads or stringly failure fields from returning.

## 2. Event & Error Inventory
Artifacts generated in `refactor/`:
- `EVENTS_INVENTORY.json` — 43 type definitions extracted from `crates/events` (post-prune).
- `event_variant_usage.json` — call-site map with 170 variant references.
- `ERRORS_INVENTORY.json` — 54 type definitions from `crates/errors`.
- `error_call_sites.json` — 154 distinct error references.

**Domain event density (top 12 by variants):**

| Event enum | Module | Variants | Used | Unused |
| --- | --- | ---: | ---: | ---: |
| AppEvent | events::mod | 16 | 16 | 0 |
| StateEvent | events::state | 7 | 7 | 0 |
| PackageEvent | events::package | 16 | 16 | 0 |
| ProgressEvent | events::progress | 14 | 12 | 2 |
| BuildEvent | events::build | 13 | 13 | 0 |
| PlatformEvent | events::platform | 12 | 12 | 0 |
| GuardEvent | events::guard | 11 | 11 | 0 |
| InstallEvent | events::install | 3 | 3 | 0 |
| DownloadEvent | events::download | 3 | 3 | 0 |
| UninstallEvent | events::uninstall | 3 | 3 | 0 |
| AuditEvent | events::audit | 8 | 8 | 0 |
| GeneralEvent | events::general | 8 | 8 | 0 |

After trimming the milestone events, the surface drops to **174** variants, with the remaining unused items concentrated in legacy progress helpers (e.g., `ProgressEvent::StatisticsUpdated`).

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
- Download/install/uninstall/state now emit only milestone triads; CLI/logging paths consume them directly.
- Progress updates continue to flow through `ProgressEvent` (35 emissions) and drive the TTY progress UI.
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
