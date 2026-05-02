# Changelog

All notable changes to this project are documented in this file.

## 0.2.0 - 2026-05-01

### Changed

- Reworked `Symbol` storage around `triomphe::ThinArc`, keeping `Symbol` pointer-sized while removing the previous hand-rolled reference-counted allocation.
- Replaced eager removal on final `Symbol` drop with deferred shard-local cleanup. Final drops now record pending cleanup work, and shards are swept periodically or through explicit cleanup calls.
- Reduced the global pool from 64 shards to 16 benchmark-tuned shards.
- Updated shard assignment to use a bounded sampling hash so long strings do not require a full scan just to choose a shard.
- Switched `Symbol` equality and hashing to interned identity. Comparisons with `str`, `&str`, `String`, and `&String` remain content-based.
- Improved mutex poisoning behavior by recovering the protected shard state instead of panicking from cleanup paths.
- Updated crate metadata, documentation, README guidance, and benchmark descriptions for the deferred cleanup model.
- Updated the crate to Rust 2024 with MSRV 1.86.

### Added

- Added `collect_unused()` to remove unused pool entries without explicitly shrinking retained shard capacity.
- Added `PartialEq<&str>`, `PartialEq<String>`, and `PartialEq<&String>` implementations for `Symbol`.
- Added broader correctness tests for cleanup, reinterning, shard selection, concurrent drops, identity hashing, and string comparisons.
- Added Miri-focused tests for repeated cleanup/reinterning and small threaded cleanup scenarios.
- Rewrote the Criterion benchmark suite around representative scenarios:
  `transient_reuse_capacity`, `cold_from_empty`, `hot_lookup`, `cleanup_drop`, and `hot_contention`.
- Added benchmark comparisons against the previous published `asylum` release, `ustr`, and plain `String` allocation.

### Removed

- Removed the old Python benchmark helper.
- Removed the old manual `Atom`/`Holder` allocation and reference-counting implementation.

### Migration Notes

- `size()` now reports entries currently retained by the pool, not necessarily only entries with live `Symbol` handles. After the final handle is dropped, an unused entry may remain until periodic cleanup, `collect_unused()`, or `shrink_to_fit()`.
- Use `collect_unused()` during quiescent maintenance periods in running programs when you want to remove unused entries while preserving capacity for future interning.
- Use `shrink_to_fit()` as a final cleanup operation, for example near the end of `main`, when you also want to release spare hash-table capacity.
- Avoid calling `shrink_to_fit()` in hot running paths unless the memory reduction is worth possible later reallocation cost.
