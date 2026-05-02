/*
 * The MIT License (MIT)
 *
 * Copyright (c) 2026 Davide Di Carlo
 *
 * Permission is hereby granted, free of charge, to any person
 * obtaining a copy of this software and associated documentation
 * files (the "Software"), to deal in the Software without
 * restriction, including without limitation the rights to use,
 * copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the
 * Software is furnished to do so, subject to the following
 * conditions:
 *
 * The above copyright notice and this permission notice shall be
 * included in all copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
 * EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES
 * OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
 * NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT
 * HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
 * WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
 * OTHER DEALINGS IN THE SOFTWARE.
 */

//! *A safe place for your strings.*
//!
//! **asylum** is a fast, lightweight, thread-safe string interner.
//!
//! It stores each unique string once, returns cheap [`Symbol`] handles, and
//! compares symbols by interned identity instead of comparing string bytes.
//! This is useful for parsers, compilers, protocol implementations, and other
//! workloads that repeatedly see the same strings.
//!
//! # Semantics
//!
//! [`Symbol`] equality and hashing are identity-based: two symbols compare
//! equal when they point to the same interned allocation. Since every live
//! allocation is canonicalized through [`intern`], symbols created from equal
//! strings compare equal while they are live. Comparisons with `str` and
//! [`String`] use string contents instead.
//!
//! `Symbol` is intentionally a small handle. Interned bytes are stored in the
//! global pool and reference counted across all live symbols.
//!
//! # Cleanup model
//!
//! Dropping the last [`Symbol`] for a string records a pending cleanup on the
//! affected shard. Once enough final drops accumulate, that shard is swept and
//! entries with no live [`Symbol`] handles are removed. This keeps `Drop` cheap
//! for short-lived symbols while bounding stale entries under continuing churn.
//!
//! Call [`collect_unused`] at quiescent points to sweep every shard without
//! explicitly shrinking capacity. Call [`shrink_to_fit`] as a final cleanup
//! operation before program shutdown when you want to release the pool's spare
//! capacity. Both functions remove entries with no live [`Symbol`] handles that
//! are observable while each shard is locked; they are exact when no concurrent
//! interning or dropping is racing with the sweep. Avoid calling
//! [`shrink_to_fit`] in a running hot path: shrinking shards can make later
//! [`intern`] calls allocate again.
//!
//! # Concurrency
//!
//! The global pool is split into independent shards. Interning a string locks
//! only the shard selected for that string; [`size`], [`capacity`], and
//! [`shrink_to_fit`] inspect all shards.

use hashbrown::HashSet;
use triomphe::ThinArc;

use std::borrow::Borrow;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

// Benchmark-tuned defaults. Sixteen shards keep contention bounded without
// spreading normal workloads across too many small hash tables. Starting at
// zero capacity avoids permanent per-shard allocation for short-lived or small
// pools; each shard grows on demand and shrink_to_fit() releases spare buckets.
const SHARD_COUNT: usize = 16;
const _: () = assert!(SHARD_COUNT.is_power_of_two());

const CLEANUP_THRESHOLD: usize = 2048;

static POOL: LazyLock<[Shard; SHARD_COUNT]> = LazyLock::new(|| std::array::from_fn(Shard::new));

struct Shard {
    entries: Mutex<HashSet<Entry>>,
    pending_drops: AtomicUsize,
    shard_index: usize,
}

impl Shard {
    fn new(shard_index: usize) -> Self {
        Self {
            entries: Mutex::new(HashSet::with_capacity(CLEANUP_THRESHOLD / SHARD_COUNT)),
            pending_drops: AtomicUsize::new(0),
            shard_index,
        }
    }

    fn intern(&self, key: &str) -> Entry {
        let mut entries = self.lock_entries();
        entries
            .get_or_insert_with(key, |k| Entry::new(self.shard_index, k))
            .clone()
    }

    fn lock_entries(&self) -> MutexGuard<'_, HashSet<Entry>> {
        // Treat mutex poisoning as recoverable.
        //
        // A panic while holding a shard lock may leave the mutex poisoned, but
        // the protected value is still memory-safe to access. For this crate,
        // continuing with the inner HashSet is preferable to panicking from
        // cleanup paths such as Symbol::drop. At worst, logical pool state is
        // repaired by later interning or by shrink_to_fit().
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn entry_count(&self) -> usize {
        self.lock_entries().len()
    }

    fn entry_capacity(&self) -> usize {
        self.lock_entries().capacity()
    }

    fn shrink_to_fit(&self) {
        self.clear_pending_drop_count();

        let mut entries = self.lock_entries();
        entries.retain(|symbol| symbol.strong_count() > 1);
        entries.shrink_to_fit();
    }

    fn collect_unused_entries(&self) {
        self.clear_pending_drop_count();
        self.remove_entries_without_live_symbols();
    }

    fn remove_entries_without_live_symbols(&self) {
        let mut entries = self.lock_entries();
        entries.retain(|entry| entry.strong_count() > 1);
    }

    fn remove_entries_without_live_symbols_while_dropping(&self, entry: &Entry) {
        let mut entries = self.lock_entries();
        entries.retain(|candidate| {
            if candidate.ptr_eq(entry) {
                // `entry` still owns its Arc while Drop is running. Remove it
                // only if the pool and this Drop frame are still the sole
                // owners.
                candidate.strong_count() > 2
            } else {
                candidate.strong_count() > 1
            }
        });
    }

    fn clear_pending_drop_count(&self) {
        self.pending_drops.swap(0, Ordering::Relaxed);
    }

    fn defer_cleanup_after_drop(&self, dropping: &Entry) {
        let mut pending = self.pending_drops.fetch_add(1, Ordering::Relaxed) + 1;

        while pending >= CLEANUP_THRESHOLD {
            match self.pending_drops.compare_exchange_weak(
                pending,
                0,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.remove_entries_without_live_symbols_while_dropping(dropping);
                    return;
                }
                Err(actual) => pending = actual,
            }
        }
    }
}

fn shard_for_key(key: &str) -> &Shard {
    &POOL[shard_index(key)]
}

fn shard_index(key: &str) -> usize {
    // This is only a shard selector, not the HashSet key hash. Keep it bounded:
    // long strings should not pay to hash every byte before the real HashSet
    // lookup hashes the key again. Sampling several stable positions gives
    // better distribution than first/middle/last while keeping cost constant.
    let bytes = key.as_bytes();
    let len = bytes.len();

    let mut hash = len as u64;
    hash = mix_sample(hash, bytes.first().copied().unwrap_or(0));
    hash = mix_sample(hash, bytes.get(len / 4).copied().unwrap_or(0));
    hash = mix_sample(hash, bytes.get(len / 2).copied().unwrap_or(0));
    hash = mix_sample(
        hash,
        bytes.get(len.wrapping_mul(3) / 4).copied().unwrap_or(0),
    );
    hash = mix_sample(hash, bytes.last().copied().unwrap_or(0));

    // MurmurHash3-style finalizer step. The multiplier is a standard
    // non-cryptographic avalanche constant; here it only spreads the sampled
    // bytes into the low bits used for shard selection.
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    hash as usize & (SHARD_COUNT - 1)
}

fn mix_sample(hash: u64, byte: u8) -> u64 {
    // Golden-ratio-style multiplicative mixer commonly used to decorrelate
    // nearby integer values in hash tables. This is not cryptographic; it only
    // helps sampled bytes affect more bits before the final shard mask.
    (hash ^ u64::from(byte)).wrapping_mul(0x9e37_79b1_85eb_ca87)
}

/// Interns a string slice and returns its canonical [`Symbol`].
///
/// If the string was already interned, returns the existing [`Symbol`].
/// Otherwise, stores the string once and returns a new [`Symbol`] pointing to
/// that interned entry.
///
/// Interning locks only the shard selected for `key`, not the whole global
/// pool.
///
/// # Example
/// ```rust
/// let sym = asylum::intern("hello");
/// assert_eq!(sym.as_str(), "hello");
/// assert_eq!(sym, "hello");
/// ```
pub fn intern(key: &str) -> Symbol {
    let entry = shard_for_key(key).intern(key);
    Symbol { entry }
}

/// Returns the number of entries currently stored in the interner.
///
/// This is the number of strings currently present in the global pool, not
/// the number of live [`Symbol`] handles. Dropping the last symbol for a string
/// can leave its entry in the pool until periodic shard cleanup,
/// [`collect_unused`], or [`shrink_to_fit`] removes it.
///
/// If you need an exact post-cleanup count at a quiescent point, call
/// [`collect_unused`] or [`shrink_to_fit`] first.
///
/// # Example
/// ```rust
/// assert_eq!(asylum::size(), 0);
///
/// let sym = asylum::intern("hello");
/// assert_eq!(asylum::size(), 1);
///
/// drop(sym);
/// asylum::shrink_to_fit();
/// assert_eq!(asylum::size(), 0);
/// ```
pub fn size() -> usize {
    POOL.iter().map(Shard::entry_count).sum()
}

/// Returns the total number of hash-table slots currently allocated by the pool.
///
/// This may be larger than [`size`] due to internal hash-set capacity growth.
/// Empty capacity is released by [`shrink_to_fit`].
pub fn capacity() -> usize {
    POOL.iter().map(Shard::entry_capacity).sum()
}

/// Collects unused pool entries without explicitly shrinking retained capacity.
///
/// Normal [`Symbol`] drops defer pool cleanup and periodically trigger a
/// shard-local cleanup pass. Calling this function explicitly sweeps every
/// shard and removes entries with no live [`Symbol`] handles.
///
/// This is intended for quiescent maintenance periods in running programs. It
/// removes entries with no live [`Symbol`] handles that are observable while
/// each shard is locked, and is exact when no concurrent interning or dropping
/// is racing with the sweep.
///
/// This function does not explicitly shrink shard capacity, so future
/// [`intern`] calls can usually reuse the existing hash-table allocation.
pub fn collect_unused() {
    POOL.iter().for_each(Shard::collect_unused_entries);
}

/// Collects unused pool entries and shrinks the interner's capacity around the
/// number of strings still referenced by live [`Symbol`]s.
///
/// Normal [`Symbol`] drops defer cleanup and trigger periodic shard-local
/// sweeps. Calling this function removes entries with no live [`Symbol`]
/// handles that are observable while each shard is locked and asks each shard
/// to release spare capacity. It is exact when no concurrent interning or
/// dropping is racing with the sweep.
///
/// Prefer calling this as the last interner operation before program shutdown,
/// for example near the end of `main`, when you want to clean up the global
/// pools completely.
///
/// Avoid calling this in a running program unless you explicitly want to trade
/// memory for time. Shrinking shards can reduce retained memory, but later
/// [`intern`] calls may need to allocate and grow those shards again.
pub fn shrink_to_fit() {
    POOL.iter().for_each(Shard::shrink_to_fit);
}

/// A lightweight handle to an interned string.
///
/// [`Symbol`] is a clonable, comparable, and hashable handle to a string stored
/// inside the interner.
///
/// Cloning a symbol is cheap because it clones a small reference-counted
/// handle. Equality between two symbols is pointer-based and therefore does
/// not scan the string contents. Comparisons with `str` and [`String`] compare
/// string contents.
///
/// Hashing a [`Symbol`] also uses interned identity, not the string bytes. This
/// makes `HashMap<Symbol, _>` and `HashSet<Symbol>` efficient, but it also
/// means `hash(symbol)` is intentionally not the same as
/// `hash(symbol.as_str())`.
pub struct Symbol {
    entry: Entry,
}

const _: [(); std::mem::size_of::<usize>()] = [(); std::mem::size_of::<Symbol>()];

impl Symbol {
    /// Creates a new [`Symbol`] for the given string slice.
    ///
    /// This method is equivalent to calling [`intern`] directly.
    ///
    /// # Example
    /// ```rust
    /// let sym = asylum::Symbol::new("hello");
    /// assert_eq!(sym.as_str(), "hello");
    /// ```
    pub fn new(key: &str) -> Self {
        intern(key)
    }

    /// Returns the interned string slice associated with this [`Symbol`].
    ///
    /// The returned `&str` is valid for as long as `self` is alive.
    ///
    /// # Example
    /// ```rust
    /// let sym = asylum::intern("hello");
    /// assert_eq!(sym.as_str(), "hello");
    /// ```
    pub fn as_str(&self) -> &str {
        self.entry.as_str()
    }

    /// Returns the number of live [`Symbol`] handles for this string.
    ///
    /// The pool itself owns one internal reference, but that reference is not
    /// included in this count.
    ///
    /// This value is primarily useful for diagnostics and tests. In concurrent
    /// code it is only a snapshot and can become stale immediately after it is
    /// read.
    ///
    /// # Example
    /// ```rust
    /// let first = asylum::intern("hello");
    /// let second = first.clone();
    ///
    /// assert_eq!(first.count(), 2);
    /// assert_eq!(second.count(), 2);
    /// ```
    pub fn count(&self) -> usize {
        self.entry.symbol_count()
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Clone for Symbol {
    fn clone(&self) -> Self {
        Self {
            entry: self.entry.clone(),
        }
    }
}

impl Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Symbol({:?})", self.as_str())
    }
}

impl Drop for Symbol {
    fn drop(&mut self) {
        if self.entry.strong_count() > 2 {
            return;
        }

        self.entry.shard().defer_cleanup_after_drop(&self.entry);
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<&String> for Symbol {
    fn eq(&self, other: &&String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<String> for Symbol {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        self.entry.ptr_eq(&other.entry)
    }
}

impl Eq for Symbol {}

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.entry.ptr_hash(state);
    }
}

#[derive(Clone)]
#[repr(transparent)]
struct Entry {
    inner: ThinArc<usize, u8>,
}

// Internal pool entry.
//
// Entry is content-hashed so the HashSet can look it up by borrowed `&str`,
// but Symbol equality and hashing use the entry allocation identity. The bytes
// live inside a thin reference-counted allocation, keeping each public Symbol
// pointer-sized while avoiding a separate Box<str> allocation.
impl Entry {
    fn new(shard_index: usize, s: &str) -> Self {
        Self {
            inner: ThinArc::from_header_and_slice(shard_index, s.as_bytes()),
        }
    }

    fn as_str(&self) -> &str {
        // Entry is only constructed from valid UTF-8 input through `new`, and
        // `ThinArc` stores an immutable byte slice.
        unsafe { std::str::from_utf8_unchecked(&self.inner.slice) }
    }

    fn shard_index(&self) -> usize {
        self.inner.header.header
    }

    fn shard(&self) -> &Shard {
        &POOL[self.shard_index()]
    }

    fn symbol_count(&self) -> usize {
        // Subtract the pool-owned reference so Symbol::count() reports only
        // public handles.
        self.strong_count().saturating_sub(1)
    }

    fn strong_count(&self) -> usize {
        ThinArc::strong_count(&self.inner)
    }

    fn ptr_eq(&self, other: &Self) -> bool {
        self.inner.as_ptr() == other.inner.as_ptr()
    }

    fn ptr_hash<H: Hasher>(&self, state: &mut H) {
        self.inner.as_ptr().hash(state);
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Entry {}

impl Hash for Entry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl Borrow<str> for Entry {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod test {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::mem;
    use std::sync::Mutex;
    use std::thread;

    static LOCK: Mutex<()> = Mutex::new(());

    fn collect_and_assert_empty() {
        crate::collect_unused();
        assert_eq!(crate::size(), 0);
    }

    #[test]
    fn symbol_is_pointer_sized() {
        assert_eq!(mem::size_of::<crate::Symbol>(), mem::size_of::<usize>());
        assert_eq!(mem::align_of::<crate::Symbol>(), mem::align_of::<usize>());
    }

    #[test]
    fn entry_hash_matches_str_hash() {
        let entry = crate::Entry::new(0, "entry_hash_matches_str_hash");

        assert_eq!(hash(&entry), hash(&"entry_hash_matches_str_hash"));
    }

    #[test]
    fn no_contention() {
        let _guard = LOCK.lock().unwrap();

        let k1 = crate::intern("no_contention_foo");
        let k2 = crate::intern("no_contention_foo");
        let k3 = k1.clone();

        assert_eq!(k1.count(), 3);
        assert_eq!(k1.as_str(), "no_contention_foo");

        assert_eq!(k2.count(), 3);
        assert_eq!(k2.as_str(), "no_contention_foo");

        assert_eq!(k3.count(), 3);
        assert_eq!(k3.as_str(), "no_contention_foo");

        assert_eq!(k1, k2);
        assert_eq!(k2, k3);
        assert_eq!(k3, k1);
        assert_eq!(crate::size(), 1);

        drop(k1);

        assert_eq!(k2.count(), 2);
        assert_eq!(k2.as_str(), "no_contention_foo");

        assert_eq!(k3.count(), 2);
        assert_eq!(k3.as_str(), "no_contention_foo");

        assert_eq!(k2, k3);
        assert_eq!(k3, k2);
        assert_eq!(crate::size(), 1);

        drop(k2);

        assert_eq!(k3.count(), 1);
        assert_eq!(k3.as_str(), "no_contention_foo");
        assert_eq!(crate::size(), 1);

        drop(k3);

        collect_and_assert_empty();

        let k4 = crate::intern("no_contention_bar");
        let k5 = crate::intern("no_contention_spam");

        assert_ne!(k4, k5);

        drop(k4);
        drop(k5);

        collect_and_assert_empty();
    }

    #[test]
    fn cleanup_and_reintern() {
        let _guard = LOCK.lock().unwrap();

        let first = crate::intern("cleanup_and_reintern");
        assert_eq!(first.count(), 1);
        assert_eq!(crate::size(), 1);
        drop(first);

        assert_eq!(crate::size(), 1);

        let second = crate::Symbol::new("cleanup_and_reintern");
        let third = crate::intern("cleanup_and_reintern");

        assert_eq!(second, third);
        assert_eq!(second.count(), 2);
        assert_eq!(third.as_str(), "cleanup_and_reintern");
        assert_eq!(crate::size(), 1);

        drop(second);
        assert_eq!(third.count(), 1);
        drop(third);

        collect_and_assert_empty();
    }

    #[test]
    fn compares_with_str_references() {
        let _guard = LOCK.lock().unwrap();

        let symbol = crate::intern("compares_with_str_references");

        assert_eq!(symbol, "compares_with_str_references");
        assert_ne!(symbol, "different");

        drop(symbol);
        collect_and_assert_empty();
    }

    #[test]
    fn compares_with_strings() {
        let _guard = LOCK.lock().unwrap();

        let symbol = crate::intern("compares_with_strings");
        let matching = String::from("compares_with_strings");
        let different = String::from("different");

        assert_eq!(symbol, matching);
        assert_ne!(symbol, different);

        let matching = String::from("compares_with_strings");
        let different = String::from("different");

        assert_eq!(symbol, &matching);
        assert_ne!(symbol, &different);

        drop(symbol);
        collect_and_assert_empty();
    }

    #[test]
    fn hash_matches_pointer_identity() {
        let _guard = LOCK.lock().unwrap();

        let first = crate::intern("hash_matches_pointer_identity");
        let second = crate::intern("hash_matches_pointer_identity");

        assert_eq!(first, second);
        assert_eq!(hash(&first), hash(&second));

        drop(first);
        drop(second);
        collect_and_assert_empty();

        let third = crate::intern("hash_matches_pointer_identity");
        assert_eq!(third.as_str(), "hash_matches_pointer_identity");

        drop(third);
        collect_and_assert_empty();
    }

    #[test]
    fn shrink_to_fit_releases_empty_capacity() {
        let _guard = LOCK.lock().unwrap();

        let symbols = (0..256)
            .map(|i| crate::intern(&format!("shrink_to_fit_releases_empty_capacity_{i}")))
            .collect::<Vec<_>>();

        assert_eq!(crate::size(), 256);
        drop(symbols);
        assert_eq!(crate::size(), 256);
        assert!(crate::capacity() > 0);

        crate::shrink_to_fit();

        assert_eq!(crate::capacity(), 0);
    }

    #[test]
    fn shard_index_uses_every_shard_for_wordlist() {
        let _guard = LOCK.lock().unwrap();
        let mut used = [false; crate::SHARD_COUNT];

        include_str!("../benches/english.txt")
            .lines()
            .for_each(|word| used[crate::shard_index(word)] = true);

        assert!(used.into_iter().all(|used| used));
    }

    #[test]
    fn shard_index_is_in_bounds_for_edge_cases() {
        let _guard = LOCK.lock().unwrap();
        let long = "very_long_string_".repeat(128);
        let keys = ["", "a", "ab", "abc", "hello", long.as_str()];

        keys.into_iter()
            .for_each(|key| assert!(crate::shard_index(key) < crate::SHARD_COUNT));
    }

    #[test]
    fn shard_index_is_reasonably_balanced_for_wordlist() {
        let _guard = LOCK.lock().unwrap();
        let mut counts = [0usize; crate::SHARD_COUNT];

        include_str!("../benches/english.txt")
            .lines()
            .for_each(|word| counts[crate::shard_index(word)] += 1);

        let total = counts.iter().sum::<usize>();
        let mean = total / crate::SHARD_COUNT;
        let min = mean / 2;
        let max = mean + mean / 2;

        counts
            .into_iter()
            .for_each(|count| assert!((min..=max).contains(&count), "{counts:?}"));
    }

    #[test]
    fn entries_remember_their_owning_shard() {
        let _guard = LOCK.lock().unwrap();
        let mut used = [false; crate::SHARD_COUNT];
        let mut symbols = Vec::with_capacity(crate::SHARD_COUNT);

        for index in 0.. {
            let key = format!("entries_remember_their_owning_shard_{index}");
            let shard_index = crate::shard_index(&key);

            if used[shard_index] {
                continue;
            }

            used[shard_index] = true;
            symbols.push(crate::intern(&key));

            if symbols.len() == crate::SHARD_COUNT {
                break;
            }
        }

        assert!(used.into_iter().all(|used| used));

        symbols.iter().for_each(|symbol| {
            assert_eq!(
                symbol.entry.shard_index(),
                crate::shard_index(symbol.as_str())
            );
            assert!(std::ptr::eq(
                symbol.entry.shard(),
                crate::shard_for_key(symbol.as_str())
            ));
        });

        drop(symbols);
        collect_and_assert_empty();
    }

    #[test]
    fn pool_retains_capacity_after_insert_until_shrink() {
        let _guard = LOCK.lock().unwrap();

        let symbol = crate::intern("pool_retains_capacity_after_insert_until_shrink");
        drop(symbol);

        assert!(crate::capacity() > 0);

        crate::shrink_to_fit();

        assert_eq!(crate::capacity(), 0);
    }

    #[test]
    fn collect_unused_removes_entries_without_explicitly_shrinking_capacity() {
        let _guard = LOCK.lock().unwrap();

        let symbols = (0..256)
            .map(|i| crate::intern(&format!("collect_unused_keeps_capacity_{i}")))
            .collect::<Vec<_>>();

        drop(symbols);
        assert_eq!(crate::size(), 256);
        let capacity = crate::capacity();
        assert!(capacity > 0);

        crate::collect_unused();

        assert_eq!(crate::size(), 0);
        assert!(crate::capacity() > 0);
        assert!(crate::capacity() <= capacity);

        crate::shrink_to_fit();
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "periodic cleanup threshold test is intentionally large and too slow under Miri"
    )]
    fn final_drops_trigger_periodic_shard_cleanup() {
        let _guard = LOCK.lock().unwrap();

        let symbols = (0..)
            .map(|i| format!("periodic_shard_cleanup_{i}"))
            .filter(|key| crate::shard_index(key) == 0)
            .take(crate::CLEANUP_THRESHOLD)
            .map(|key| crate::intern(&key))
            .collect::<Vec<_>>();

        assert_eq!(crate::size(), crate::CLEANUP_THRESHOLD);
        drop(symbols);

        assert_eq!(crate::size(), 0);
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "covered by miri::small_threaded_cleanup with a much smaller workload"
    )]
    fn contention() {
        let _guard = LOCK.lock().unwrap();

        let seeds = [
            "contention_foo",
            "contention_bar",
            "contention_spam",
            "contention_lorem",
            "contention_ipsum",
            "contention_dolor",
        ];
        let t1 =
            thread::spawn(move || seeds.iter().copied().map(crate::intern).collect::<Vec<_>>());
        let t2 =
            thread::spawn(move || seeds.iter().copied().map(crate::intern).collect::<Vec<_>>());

        let s3 = seeds.iter().copied().map(crate::intern).collect::<Vec<_>>();
        let s2 = t2.join().unwrap();
        let s1 = t1.join().unwrap();

        seeds
            .iter()
            .zip(&s1)
            .zip(&s2)
            .zip(&s3)
            .for_each(|(((&seed, s1), s2), s3)| {
                assert_eq!(s1.count(), 3);
                assert_eq!(s2.count(), 3);
                assert_eq!(s3.count(), 3);
                assert_eq!(s1, s2);
                assert_eq!(s2, s3);
                assert_eq!(s3, s1);
                assert_eq!(seed, s1.as_str());
            });

        assert_eq!(crate::size(), seeds.len());

        drop(s1);
        drop(s2);
        drop(s3);

        collect_and_assert_empty();
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "stress loop is covered by smaller dedicated Miri tests"
    )]
    fn concurrent_drop_and_reintern() {
        let _guard = LOCK.lock().unwrap();

        for round in 0..128 {
            let key = format!("concurrent_drop_and_reintern_{round}");
            let symbol = crate::intern(&key);
            let clone = symbol.clone();
            let expected = key.clone();

            let handle = thread::spawn(move || {
                assert_eq!(clone.as_str(), expected);
                drop(clone);
            });

            assert_eq!(symbol.as_str(), key);
            handle.join().unwrap();
            assert_eq!(symbol.count(), 1);
            drop(symbol);
            assert_eq!(crate::size(), 1);

            let reinterned = crate::intern(&key);
            assert_eq!(reinterned.as_str(), key);
            assert_eq!(reinterned.count(), 1);
            drop(reinterned);
            collect_and_assert_empty();
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "thread-heavy stress loop is covered by smaller dedicated Miri tests"
    )]
    fn shrink_to_fit_collects_after_concurrent_final_drops() {
        let _guard = LOCK.lock().unwrap();

        for round in 0..128 {
            let key = format!("shrink_to_fit_collects_after_concurrent_final_drops_{round}");
            let symbols = (0..16).map(|_| crate::intern(&key)).collect::<Vec<_>>();
            assert_eq!(crate::size(), 1);

            let handles = symbols
                .into_iter()
                .map(|symbol| thread::spawn(move || drop(symbol)))
                .collect::<Vec<_>>();

            for handle in handles {
                handle.join().unwrap();
            }

            crate::shrink_to_fit();
            assert_eq!(crate::size(), 0);
        }
    }

    #[cfg(miri)]
    mod miri {
        #[test]
        fn repeated_reintern_after_cleanup() {
            let _guard = super::LOCK.lock().unwrap();

            for i in 0..4 {
                let key = format!("miri_repeated_reintern_after_cleanup_{i}");
                let symbol = crate::intern(&key);
                let clone = symbol.clone();

                assert_eq!(symbol, clone);
                assert_eq!(symbol.as_str(), key);

                drop(symbol);
                assert_eq!(clone.count(), 1);
                drop(clone);
                super::collect_and_assert_empty();

                let reinterned = crate::intern(&key);
                assert_eq!(reinterned.as_str(), key);
                drop(reinterned);
                super::collect_and_assert_empty();
            }
        }

        #[test]
        fn small_threaded_cleanup() {
            let _guard = super::LOCK.lock().unwrap();

            let key = "miri_small_threaded_cleanup";
            let symbol = crate::intern(key);
            let clone = symbol.clone();

            let handle = std::thread::spawn(move || {
                assert_eq!(clone.as_str(), key);
                drop(clone);
            });

            handle.join().unwrap();
            assert_eq!(symbol.count(), 1);
            drop(symbol);
            super::collect_and_assert_empty();
        }
    }

    fn hash<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }
}
