# Changelog

All notable changes to this project are documented in this file.

## 0.3.0 - 2026-11-22

### Added

- Added `asylum::with` and `asylum::with_shards` for interning dynamic text in an independent scope. Scoped interners accept 1 through 128 shards, support concurrent interning, and free all their entries when the callback returns or unwinds.
- Added `asylum::intern_static` to borrow static input bytes when creating a global entry. It shares the registry with `asylum::intern`.
- Added `Display` for `Symbol` and support for accessing the text through `as_str()` for the lifetime of the entry.

### Changed

- The global `intern(&str)` function now returns `Symbol<'static>` and retains each entry for the life of the process.
- `Symbol<'a>` is a pointer-sized, `Copy` handle tied to its interner's lifetime. Symbols from the same interner continue to compare and hash by identity. Use `as_str()` for content comparisons across interners.
- Both interners use eight lookup shards by default. Scoped entries are stored in a `boxcar` arena, replacing reference-counted `triomphe` storage.
- `size()` counts distinct entries. `capacity()` now reports the total entry count the lookup tables can hold without growing regardless of which shard receives new entries. `shrink_to_fit()` shrinks the lookup tables and preserves all entries.
- Registry operations panic if they encounter a poisoned shard mutex.
- Updated the Criterion suite to compare scoped insertion and teardown, plus hot lookup and contention in both APIs, with the applicable asylum 0.2, `ustr`, and `String` baselines.

### Removed

- Removed `collect_unused()`, `Symbol::new()`, `Symbol::count()`, and drop-triggered cleanup.

### Migration Notes

- Replace `Symbol::new` with `asylum::intern` for global entries. Use `asylum::intern_static` to borrow static input bytes on insertion and `Symbol<'static>` in type signatures that store global symbols.
- To free dynamic text when a workload finishes, use `interner.intern(&text)` inside `asylum::with`. Return `symbol.as_str().to_owned()` if the text needs to leave the scope.
- Use `shrink_to_fit()` when you want to reduce lookup-table capacity while keeping the entries.

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
