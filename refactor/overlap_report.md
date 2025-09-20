# Event/Error Overlap & Redundancy Findings

## Snapshot Metrics
- **207** event enum variants defined across 43 exported enums; **49 (24%)** remain without call-sites after the Phase 1 trims (`event_variant_usage.json`). The stragglers live almost entirely inside the dormant `PythonEvent` domain and legacy `ProgressEvent` helpers.
- **229** error enum variants across 13 enums; **93 (41%)** still have zero call-site references (`error_call_sites.json`).
- Progress-style data now appears in **12** domain variants (down from 45) thanks to pruning, but the gap between progress streams and domain milestones persists.

## Progress Leakage Into Domain Events
- `DownloadEvent::{Progress,BatchProgress,SpeedUpdate,Stalled}` duplicate the granular metrics already modeled by `ProgressEvent::{Updated,BatchProgress,StatisticsUpdated,BottleneckDetected}`.
- `InstallEvent` and `AcquisitionEvent` expose phase/step updates (`StagingStarted`, `ValidationStarted`, `BatchAcquisitionProgress`, etc.) that the CLI only logs; most variants are never emitted, and none drive the progress UI.
- Progress IDs (`id`, `phase`, `operation`) are recomputed per call-site instead of flowing through a shared `EventMeta`, making it impossible to correlate domain milestones with progress updates.

## Error Semantics Bleeding Into Events
- Nearly every domain event has a terminal `*Failed { error: String, .. }` variant. This transports raw strings instead of the structured `sps2_errors::Error`; consumers cannot distinguish retryable vs fatal conditions.
- Examples: `InstallEvent::Failed`, `DownloadEvent::Failed`, `GuardEvent::HealingFailed`, `UpdateEvent::Failed`, `ProgressEvent::Failed`. Payloads carry fields such as `error_category`, `phase`, `cleanup_required` that mirror data already present in the corresponding error enums.
- The CLI does not branch on these variants; it logs the string and then relies on the propagated `Result<T, Error>`, so the extra event adds noise without signal.

## Dead & Partially Dead Surface Area
- Most high-churn domains now map 1:1 with consumers; the remaining dead surface is concentrated in `PythonEvent` (10 variants, zero emitters) and the lower-case `ProgressEvent::*` helpers.
- `ProgressEvent` itself is still not pattern-matched in the CLI, so tracker signals from `ProgressManager` continue to be dropped on the floor.

## Payloads With No Consumers
- Remaining payload-heavy variants are limited to analytics-style data that CLI currently ignores (`ProgressEvent::StatisticsUpdated`, `GuardEvent::ErrorSummary`, etc.); the next phase should either wire them into telemetry or slim them further.

## Error Taxonomy Overlap
- Domain error enums (`InstallError`, `BuildError`, `NetworkError`) already encode retryability and failure context. Emitting parallel `*Failed` events with string payloads duplicates this information and locks clients out of structured handling.
- The top-level `Error` enum still permits ad-hoc constructors (`internal`, `custom`, `Io`) that do not carry durable codes or remediation guidance, so progress/error streams cannot provide actionable hints.
- Many domain error variants differ only in phrasing (e.g., `BuildError::{DraftMetadataFailed,DraftSourceFailed,RecipeError}`) yet have no dedicated handling; 93 variants have zero references.

## Consequences
- **Complexity**: Engineers must sift through hundreds of unused variants to understand the contract, heightening the chance of incorrect emissions.
- **Inconsistency**: Progress, milestones, and failures are expressed through overlapping enums, making it impossible for consumers to reconstruct a linear story of an operation.
- **Performance drag**: Large payloads (Vecs, PathBufs, Duration) are cloned and serialized even though receivers drop them, inflating the event bus and log volume.
- **Observability gaps**: Despite the breadth of enums, core flows (install/update) expose neither correlation IDs nor durable error codes, so UI/logs cannot stitch together user-facing narratives.

## Quick Wins Identified
1. Finish collapsing the dormant `PythonEvent` family and the lower-case `ProgressEvent::*` helpers, then guard against regressions with a lint/check.
2. Route progress information exclusively through a slim `ProgressEvent` (or forthcoming `ProgressUpdate`) channel and strip the remaining progress-shaped variants.
3. Replace `error: String` payloads with structured error references (`ErrorCode`, `retryable`) or rely on `Result<T, Error>` propagation; reserve events for milestones (`InstallCompleted`, `InstallRolledBack`).
4. Introduce a shared `EventMeta` (id, source component, severity, correlation id) carried by every event to enable cross-stream correlation and downstream filtering.
