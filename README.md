# Asylum

[![Crates.io](https://img.shields.io/crates/v/asylum.svg)](https://crates.io/crates/asylum)
[![Docs.rs](https://img.shields.io/docsrs/asylum)](https://docs.rs/asylum)
[![License](https://img.shields.io/crates/l/asylum.svg)](https://github.com/daddinuz/asylum/blob/main/LICENSE)

*A safe place for your strings.*

**asylum** is a fast, lightweight string interner with automatic cleanup to prevent memory bloat.

It stores each unique string once, supports fast equality checks,
and automatically removes unused strings to keep memory usage low.

Whether you're building compilers, parsers, or any project dealing with lots of duplicate strings,
**asylum** provides a simple, reliable solution.

---

## Features

- **Fast and lightweight**: Designed for high-throughput applications.
- **Memory-efficient**: Only one copy of each string is stored.
- **Automatic cleanup**: Strings are reference-counted and collected when no longer in use.
- **Simple API**: Easy to integrate into any Rust project.
- **Thread-safe**: Works in multi-threading contexts.

## Iternals

Under the hood, **asylum** uses a global pool of HashMap(s) to store interned strings efficiently.
Each string is reference-counted, and when the last reference to a string is dropped, the string is automatically removed from the interner.

This ensures that memory usage stays under control, even in long-running applications where many transient strings are interned.

Compared to other interners like `ustr`, which keep strings alive indefinitely after interning, asylum dynamically cleans up
unused strings to prevent memory bloat without sacrificing lookup performance.

## When to use

Use asylum when:
- You need fast pointer-based equality checks for strings.
- You expect many repeated or short-lived strings.
- You want automatic memory cleanup without manually managing lifetimes.

## How to use

```rust
use asylum;

fn main() {
  // intern strings into atomic symbols
  let hello1 = asylum::intern("hello");
  let hello2 = asylum::intern("hello");

  // same memory for identical strings
  assert_eq!(hello1, hello2));
  assert_eq!(asylum::size(), 1);

  // you can obtain the actual string with `as_str()`
  assert_eq!(hello1.as_str(), "hello");

  // when all references to "hello" are dropped, symbols are automatically collected
  drop(hello1);
  assert_eq!(asylum::size(), 1);
  drop(hello2);
  assert_eq!(asylum::size(), 0);

  // symbols are collected but the slots in the pool
  // won't, this makes future string interning more efficient
  assert!(asylum::capacity() > 0);

  // but you can safely trigger a cycle of collection any time
  asylum::shrink_to_fit();
  assert_eq!(asylum::capacity(), 0);
}
```

---

## Installation

Add **asylum** to your `Cargo.toml`:

```toml
[dependencies]
asylum = "0.1"
```

## Benchmarks

You can run benchmarks using cargo:
```sh
cargo bench
```

The comparison is currently made against `ustr` crate which uses a similar architecture. 

Generally speaking, this crate is as performant as `ustr` even if it has to handle reference counting of interned strings.
Regarding memory usage, this crate may end up performing more small-size allocations, but memory is automatically reclaimed
when symbols are not used anymore and manual grabage collection can be performed for the global pool. 

## LICENSE

**asylum** is licensed under MIT terms.
