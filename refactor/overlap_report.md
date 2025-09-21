# Event/Error Overlap & Redundancy Findings

## Snapshot Metrics
- **174** event enum variants defined across 42 exported enums after trimming download/install/uninstall/state; roughly **30** remain without call-sites (primarily legacy `ProgressEvent` helpers and acquisition/resolver progress variants).
- **229** error enum variants across 13 enums; **93 (41%)** still have zero call-site references (`error_call_sites.json`).
- Progress-style data is now confined to acquisition/resolver batches and a few analytics helpers; core lifecycle domains emit only milestone triads.

## Progress Leakage Into Domain Events
- Acquisition/resolver still publish batch-oriented progress variants (`BatchAcquisitionProgress`, `ResolverEvent::ResolutionProgress`) even though the CLI ignores them in favour of `ProgressEvent`.
- Remaining `ProgressEvent::*` helpers such as `StatisticsUpdated` and `BottleneckDetected` are emitted rarely and have no consumers; they can be removed once telemetry needs are documented.

## Error Semantics Bleeding Into Events
- Several domains now expose `retryable` booleans instead of raw strings (download/install/uninstall), but acquisition/guard/repo still emit `error: String` payloads that duplicate information already present in `sps2_errors`.
- `ProgressEvent::Failed` continues to carry a string payload; consumers rely on the accompanying `Result<T, Error>` to surface hints, so the field adds noise without signal.

## Dead & Partially Dead Surface Area
- High-churn domains (download/install/uninstall/state) now map 1:1 with consumers; the remaining dead surface is concentrated in dormant progress helpers and acquisition/resolver batch events.
- `ProgressEvent` updates feed the CLI progress renderer, but analytics-only helpers (statistics/bottleneck) remain unused and should be pruned or moved behind telemetry flags.

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
- **Observability gaps**: Durable failure context is still stringly, so even with rendered progress and correlation IDs the UI/logs struggle to surface retryability or remediation guidance.

## Quick Wins Identified
1. Collapse acquisition/resolver progress batches to `ProgressEvent`, matching the triads now used elsewhere.
2. Drop the unused `ProgressEvent::{StatisticsUpdated,BottleneckDetected}` helpers or move them behind telemetry-specific features.
3. Replace remaining `error: String` payloads in acquisition/guard/repo events with retryability flags or structured context derived from `sps2_errors`.
4. Regenerate the inventories so contributors no longer see the removed variants (Python, staging/validation) in the analysis artefacts.
