// SPDX-License-Identifier: MIT

// A separate test binary gives these assertions a fresh global registry.
#[test]
fn global_registry_tracks_distinct_text_and_preserves_symbols_when_shrunk() {
    assert_eq!(asylum::size(), 0);
    asylum::shrink_to_fit();

    let first = asylum::intern(&String::from("first"));
    let second = asylum::intern_static("second");
    assert_eq!(first, asylum::intern_static("first"));
    assert_eq!(asylum::size(), 2);
    // A new key can land in an empty shard with no allocated capacity.
    assert_eq!(asylum::capacity(), 2);

    asylum::shrink_to_fit();

    assert_eq!(asylum::size(), 2);
    assert_eq!(asylum::capacity(), 2);
    assert_eq!(first, asylum::intern_static("first"));
    assert_eq!(second, asylum::intern("second"));
}
