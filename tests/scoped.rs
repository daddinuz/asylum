// SPDX-License-Identifier: MIT

use std::collections::HashSet;
use std::panic::catch_unwind;
use std::sync::Barrier;
use std::thread;

#[test]
fn scoped_symbol_survives_growth_and_outlives_the_interner_handle() {
    asylum::with(|interner| {
        let input = String::from("scoped text");
        let symbol = interner.intern(&input);
        drop(input);

        for index in 0..256 {
            interner.intern(&index.to_string());
        }
        assert_eq!(symbol, interner.intern("scoped text"));

        drop(interner);
        assert_eq!(symbol.as_str(), "scoped text");
    });
}

#[test]
fn equal_text_reuses_the_same_hash_key() {
    asylum::with(|interner| {
        let first = interner.intern("name");
        let duplicate = interner.intern(&String::from("name"));
        let distinct = interner.intern("names");
        assert_eq!(first, duplicate);
        assert_ne!(first, distinct);

        let symbols = HashSet::from([first, duplicate, distinct]);
        assert_eq!(symbols.len(), 2);
        assert!(symbols.contains(&interner.intern("name")));
    });
}

#[test]
fn symbol_compares_with_borrowed_and_owned_text() {
    asylum::with(|interner| {
        let symbol = interner.intern("name");
        let same = String::from("name");
        let different = String::from("names");

        assert_eq!(symbol, same.as_str());
        assert_eq!(symbol, &same);
        assert_eq!(symbol, same);
        assert_ne!(symbol, different.as_str());
        assert_ne!(symbol, &different);
        assert_ne!(symbol, different);
    });
}

#[test]
fn scoped_registry_tracks_distinct_text_and_preserves_symbols_when_shrunk() {
    asylum::with(|interner| {
        assert_eq!(interner.size(), 0);
        interner.shrink_to_fit();

        let first = interner.intern("first");
        let second = interner.intern("second");
        assert_eq!(first, interner.intern("first"));
        assert_eq!(interner.size(), 2);
        assert_eq!(interner.capacity(), 2);

        interner.shrink_to_fit();

        assert_eq!(interner.size(), 2);
        assert_eq!(interner.capacity(), 2);
        assert_eq!(first, interner.intern("first"));
        assert_eq!(second, interner.intern("second"));
    });
}

#[test]
fn cached_lookup_at_capacity_does_not_grow_table() {
    asylum::with_shards::<1, _>(|interner| {
        let first = interner.intern("first");
        let capacity = interner.capacity();
        assert!(capacity > interner.size());

        for index in 1..capacity {
            interner.intern(&index.to_string());
        }

        assert_eq!(interner.size(), capacity);
        assert_eq!(first, interner.intern("first"));
        assert_eq!(interner.capacity(), capacity);
    });
}

#[test]
fn scoped_registry_accepts_a_non_power_of_two_shard_count() {
    asylum::with_shards::<3, _>(|interner| {
        for index in 0..32 {
            let input = index.to_string();
            let symbol = interner.intern(&input);
            assert_eq!(symbol, interner.intern(&input));
        }

        assert_eq!(interner.size(), 32);
        assert!(interner.capacity() >= interner.size());
    });
}

#[test]
fn scoped_registry_accepts_128_shards() {
    asylum::with_shards::<128, _>(|interner| {
        let symbol = interner.intern("limit");
        assert_eq!(symbol, interner.intern("limit"));
        assert_eq!(interner.size(), 1);
    });
}

#[test]
fn preserves_empty_text_unicode_and_embedded_nuls() {
    asylum::with(|interner| {
        let inputs = ["", "\u{e9}", "e\u{301}", "a\0b", "ab"];
        let mut symbols = HashSet::new();

        for input in inputs {
            let symbol = interner.intern(input);
            assert_eq!(symbol.as_str(), input);
            assert_eq!(symbol, interner.intern(input));
            assert!(symbols.insert(symbol));
        }
    });
}

#[test]
fn nested_scopes_can_compare_text_and_return_owned_data() {
    let owned = asylum::with(|outer| {
        let outer_text = outer.intern("nested text").as_str();

        asylum::with(|inner| {
            let inner_text = inner.intern(outer_text).as_str();
            assert_eq!(outer_text, inner_text);
            inner_text.to_owned()
        })
    });

    assert_eq!(owned, "nested text");
}

#[test]
fn shared_interner_deduplicates_concurrent_inserts() {
    const WORKERS: usize = 4;

    asylum::with(|interner| {
        let barrier = Barrier::new(WORKERS);
        let interner = &interner;
        let barrier = &barrier;

        thread::scope(|scope| {
            let workers: Vec<_> = (0..WORKERS)
                .map(|worker| {
                    scope.spawn(move || {
                        barrier.wait();
                        let shared = interner.intern("shared text");

                        for index in 0..32 {
                            let input = format!("{worker}/{index}");
                            assert_eq!(interner.intern(&input).as_str(), input);
                        }

                        shared
                    })
                })
                .collect();

            interner.shrink_to_fit();

            let symbols: HashSet<_> = workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect();
            assert_eq!(symbols, HashSet::from([interner.intern("shared text")]));
            assert_eq!(interner.size(), 1 + WORKERS * 32);
        });
    });
}

#[test]
fn callback_panic_propagates_after_interning() {
    let panic = catch_unwind(|| {
        asylum::with(|interner| {
            for index in 0..64 {
                interner.intern(&index.to_string());
            }
            panic!("scoped callback failed");
        });
    });

    let payload = panic.unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"scoped callback failed")
    );
}
