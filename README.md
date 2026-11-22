# Asylum

[![Crates.io](https://img.shields.io/crates/v/asylum.svg)](https://crates.io/crates/asylum)
[![Docs.rs](https://img.shields.io/docsrs/asylum)](https://docs.rs/asylum)
[![License](https://img.shields.io/crates/l/asylum.svg)](https://github.com/daddinuz/asylum/blob/main/LICENSE)

*A safe place for your strings.*

**asylum** is a thread-safe string interner.
It stores each distinct string once per interner and returns pointer-sized, copyable `Symbol` handles.
Symbols from the same interner compare and hash by entry identity in constant time.

## Installation

```toml
[dependencies]
asylum = "0.3"
```

## Scoped interning

Use `asylum::with` for dynamic strings that belong to a request, parse, or other bounded workload.
The interner copies each new string into scope-owned storage and frees all entries when the callback returns or unwinds.
Symbols remain valid throughout the callback, even if the input string or interner handle is dropped.
Return owned text if you need to use it outside the callback.

```rust
let name = asylum::with(|interner| {
    let input = String::from("request");
    let first = interner.intern(&input);
    let second = interner.intern("request");

    assert_eq!(first, second);
    assert_eq!(interner.size(), 1);

    first.as_str().to_owned()
});

assert_eq!(name, "request");
```

A scoped interner uses eight shards by default.
Use `asylum::with_shards::<N, _>` to choose between 1 and 128 shards.
Threads can share the interner by reference within the scope.

## Global interning

Use `asylum::intern` to intern any `&str` for the life of the program.
It copies the input when creating an entry.
For static text such as keywords, `asylum::intern_static` can create an entry that borrows the input bytes.
Both functions use the same global interner and return the same `Symbol<'static>` for equal text.

```rust
let input = String::from("request");
let name = asylum::intern(&input);
drop(input);

let text: &'static str = name.as_str();
assert_eq!(text, "request");

let keyword = asylum::intern_static("let");
assert_eq!(keyword, asylum::intern("let"));
```

## Symbols and lookup tables

- To compare text across interners, use `as_str()`. A symbol also compares by contents with a `&str`, `String`, or `&String` on the right of `==`.
- `size()` reports the number of distinct entries in the selected interner.
- `capacity()` reports the number of distinct entries the lookup tables can hold without growing, regardless of which shard receives the next insertions. It is the current size plus the least headroom among the shards. Text storage grows separately.
- `shrink_to_fit()` shrinks the lookup tables around their current entries and preserves all symbols.

## Benchmarks

Run the Criterion suite with `cargo bench`. The benchmarks compare asylum 0.3 with asylum 0.2, `ustr`, and owned `String` across short, duplicate-heavy, bounded 8–64 byte, and long 128–256 byte strings.

- `cold_from_empty` times filling a fresh scoped interner, an emptied 0.2 pool, or a cleared `ustr` cache. Reset and teardown are outside the measurement. The `String` baseline copies the inputs.
- `hot_lookup` times repeated lookups in pre-populated interners, including both the scoped and global APIs. The `String` baseline still copies the inputs.
- `cleanup_drop` times freeing a populated scope, dropping the 0.2 handles and shrinking its pool, or dropping owned strings.
- `hot_contention` runs the hot workload concurrently on four threads.

## License

Asylum is licensed under the [MIT License](LICENSE).
