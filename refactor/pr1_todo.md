# PR1 – Surface Simplification (Work Items)

- [x] Add `EventMeta`, `EventLevel`, and `EventSource` scaffolding (`crates/events/src/meta.rs`).
- [x] Introduce structured error envelope (`StructuredError`, `ErrorCode`, `ErrorSeverity`, `ErrorContext`).
- [x] Prune unused `DownloadEvent` variants and adjust re-exports/log-level mapping.
- [x] Identify additional unused variants in other domains (install/update/resolver) for removal or deprecation (produce list from inventory).
- [x] Prune unused variants across acquisition/audit/build/general/guard/install/package/repo/resolver/state/uninstall/update events.
- [ ] Update documentation (`refactor/report.md`) to reflect the pared-down surface and remaining candidates.
- [x] Run `cargo fmt` + targeted `cargo check -p sps2-events` / `-p sps2-errors` to ensure scaffolding compiles.
