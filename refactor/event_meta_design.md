# EventMeta & Structured Error Design

## Event Metadata (crates/events/src/meta.rs)
- `EventMeta` now encapsulates identifiers (`event_id`, `parent_id`), correlation ids, timestamps, severity, origin, and opt-in labels.
- `EventLevel` is a lightweight severity enum that round-trips with `tracing::Level`.
- `EventSource` is a thin wrapper over `Cow<'static, str>` with predefined constants for each domain (general/download/install/etc.) and a helper for dynamic sources.
- Builder helpers expose expressive APIs (`with_correlation_id`, `with_parent`, `with_label`).

### Emission expectations
- All event APIs now emit `EventMessage { meta, event }` envelopes; helper constructors ensure a fresh `EventMeta` is attached when call-sites do not provide one explicitly.
- `EventEmitter` exposes `emit_with_meta`, and the default `emit` path auto-generates metadata (ids, levels, correlation) before forwarding to the sender.
- CLI/logging consume the metadata directly, relying on `EventMeta::tracing_level()` / `EventSource::as_str()` for structured output and span correlation.
- The consolidated `ProgressManager` emits `ProgressEvent`s through the same envelope, with deterministic `parent_id`/`correlation_id` values that drive the TTY progress UI and JSON output.

## Error Handling Direction
- Domain error enums remain simple `thiserror` enums returning descriptive messages.
- The root `Error` type wraps domain enums and a handful of ad-hoc cases (`Internal`, `Cancelled`, `Io`).
- The previous `StructuredError` experiment is no longer in use; any future enrichment should focus on lightweight helpers or trait-based formatting rather than envelope types.

### Integration strategy
1. Prefer returning domain errors directly; only upcast to the root `Error` at cross-crate boundaries.
2. Keep CLI rendering straightforward (`Display` derived from the error enums). Add helper formatting only if it materially improves UX without adding heavy machinery.
3. Ensure events that surface failures carry context by including relevant fields in the event payload, not via auxiliary error envelopes.
4. Guard-specific diagnostics (context builders, summaries, hints) now live alongside the guard implementation in `crates/guard/src/diagnostics.rs`; the shared errors crate only defines the lightweight `GuardError` enum and `DiscrepancySeverity` type.
