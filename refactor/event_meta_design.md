# EventMeta & Structured Error Design

## Event Metadata (crates/events/src/meta.rs)
- `EventMeta` now encapsulates identifiers (`event_id`, `parent_id`), correlation ids, timestamps, severity, origin, and opt-in labels.
- `EventLevel` is a lightweight severity enum that round-trips with `tracing::Level`.
- `EventSource` is a thin wrapper over `Cow<'static, str>` with predefined constants for each domain (general/download/install/etc.) and a helper for dynamic sources.
- Builder helpers expose expressive APIs (`with_correlation_id`, `with_parent`, `with_label`).

### Emission expectations
- All new event APIs will accept or generate an `EventMeta`.
- `EventEmitter` will grow an `emit_with_meta` helper and default `emit` will forward an auto-generated `EventMeta` with reasonable defaults.
- CLI/logging can rely on `EventMeta::tracing_level()` / `EventSource::as_str()` for structured output.

## Structured Error Envelope (crates/errors/src/structured.rs)
- `StructuredError` provides a durable `ErrorCode`, `ErrorSeverity`, friendly message, optional details, metadata map, and an `ErrorContext` block.
- `ErrorCode::as_str()` surfaces stable identifiers (e.g. `PM0201`) for UI and telemetry.
- `ErrorContext` captures operation/package/resource hints plus arbitrary labels and human hints.
- Builders (`with_details`, `with_metadata`, `with_context`) keep construction ergonomic while encouraging small payloads.

### Integration strategy
1. Domain error enums will grow conversions to `StructuredError`, mapping existing variants to standardised codes/severity.
2. High-level operations (`ops`, `install`, `resolver`) will return `StructuredError` (or wrap them in `Error`) enabling the CLI to display codes + hints.
3. Event failure variants will carry `ErrorCode` (or the entire `StructuredError`) instead of raw strings.
