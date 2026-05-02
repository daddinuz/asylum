# Asylum

[![Crates.io](https://img.shields.io/crates/v/asylum.svg)](https://crates.io/crates/asylum)
[![Docs.rs](https://img.shields.io/docsrs/asylum)](https://docs.rs/asylum)
[![License](https://img.shields.io/crates/l/asylum.svg)](https://github.com/daddinuz/asylum/blob/main/LICENSE)

*A safe place for your strings.*

**asylum** is a fast, lightweight, thread-safe string interner with deferred cleanup.

It stores each unique string once, returns cheap `Symbol` handles, supports fast identity-based equality checks, and can reclaim unused interned strings at deferred cleanup points.

It is intended for compilers, parsers, protocol implementations, and other workloads that repeatedly see the same strings, especially when many strings are short-lived.

---

## Features

- **Fast and lightweight**: Designed for high-throughput applications.
- **Memory-efficient**: Only one copy of each string is stored.
- **Cheap symbols**: `Symbol` is a small reference-counted handle to interned bytes.
- **Identity equality**: `Symbol` equality and hashing use interned identity instead of scanning string bytes.
- **Deferred cleanup**: Final `Symbol` drops stay cheap and trigger periodic shard-local cleanup.
- **Explicit sweeping**: `collect_unused()` removes entries with no live `Symbol` handles, while `shrink_to_fit()` also releases spare capacity.
- **Simple API**: Easy to integrate into any Rust project.
- **Thread-safe**: The global pool is sharded to reduce lock contention.

## Internals

Under the hood, **asylum** uses a sharded global pool of hash sets. Each pool entry stores the interned bytes in a thin reference-counted allocation. The pool owns one reference, and every live `Symbol` owns another reference to the same entry.

When the last `Symbol` for a string is dropped, asylum records a pending cleanup on the affected shard instead of removing the entry immediately. Once enough final drops accumulate on that shard, asylum sweeps it and removes entries with no live `Symbol` handles. Calling `collect_unused()` explicitly sweeps every shard without shrinking retained capacity. Calling `shrink_to_fit()` performs the same cleanup and also shrinks retained hash-table capacity.

Explicit cleanup functions remove entries observable while each shard is locked. They are exact at quiescent points, when no concurrent interning or dropping is racing with the sweep.

Compared to interners such as `ustr`, which keep interned strings alive indefinitely, asylum is designed to reclaim unused strings and provide an explicit maintenance point for long-running applications.

## Semantics

`Symbol` equality is identity-based. If two symbols are live handles to the same interned string, they compare equal without comparing string bytes. Comparisons with `str` and `String` compare contents.

`Hash` for `Symbol` is also identity-based. This makes `HashSet<Symbol>` and `HashMap<Symbol, _>` efficient, but it intentionally means `hash(symbol)` is not the same as `hash(symbol.as_str())`.

`size()` reports the number of entries currently stored in the global pool. It is not the number of live `Symbol` handles. `capacity()` reports retained hash-table capacity across all shards.

## When to use

Use asylum when:
- You need fast pointer-based equality checks for strings.
- You expect many repeated or short-lived strings.
- You want best-effort deferred cleanup plus an explicit sweep for long-running applications.
- You can use a global process-wide interner.

Avoid asylum when:
- You need independent per-context interners.
- You need content-based `Hash` for `Symbol`.
- You want interned strings to intentionally live for the full process lifetime.

## How to use

```rust
use asylum;

fn main() {
    // Intern strings into symbols.
    let hello1 = asylum::intern("hello");
    let hello2 = asylum::intern("hello");

    // Equal strings map to the same interned entry while live.
    assert_eq!(hello1, hello2);
    assert_eq!(asylum::size(), 1);

    // You can always read the interned string.
    assert_eq!(hello1.as_str(), "hello");
    assert_eq!(hello1, "hello");

    // Dropping one handle leaves the entry alive because another handle exists.
    drop(hello1);
    assert_eq!(hello2.count(), 1);
    assert_eq!(asylum::size(), 1);

    // Dropping the final handle can leave an unused entry until deferred
    // cleanup runs.
    drop(hello2);

    // Run the final cleanup at a quiescent point to remove entries with no
    // live Symbol handles and release retained capacity.
    asylum::shrink_to_fit();
    assert_eq!(asylum::size(), 0);
    assert_eq!(asylum::capacity(), 0);
}
```

---

## Installation

Add **asylum** to your `Cargo.toml`:

```toml
[dependencies]
asylum = "0.2"
```

## Benchmarks

You can run benchmarks with Cargo:

```sh
cargo bench
```

The benchmark suite compares the current checkout with the previous published `asylum` release, `ustr`, and plain `String` allocation across short strings, duplicate-heavy strings, bounded `<=64` byte strings, cleanup/drop costs, a long-string stress case, and a small contention workload.

The transient benchmarks are split by reset policy:

- `transient_reuse_capacity`: current `asylum` calls `collect_unused()` between iterations, so unused entries are removed while shard capacity is retained. This models repeated transient batches in a running process. The previous `asylum` release removes final entries eagerly, `ustr` clears its cache between iterations, and `String` allocates fresh owned strings.
- `cold_from_empty`: current `asylum` calls `shrink_to_fit()` between iterations, so each iteration starts from an empty pool with released capacity. The previous `asylum` release also calls `shrink_to_fit()`, `ustr` clears its cache, and `String` allocates fresh owned strings.
- `hot_lookup`: each interner is pre-populated before measurement, then the benchmark measures repeated lookup/intern calls for already-seen strings.
- `cleanup_drop`: measures the cost of dropping the final handles after setup has interned the workload.
- `hot_contention`: measures repeated intern calls from multiple threads against a pre-populated workload.

Always benchmark with your own workload before choosing an interner. The main tradeoff is that asylum pays reference-counting and cleanup costs in exchange for reclaiming interned strings that are no longer used.

## LICENSE

**asylum** is licensed under MIT terms.
