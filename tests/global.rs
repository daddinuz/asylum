// SPDX-License-Identifier: MIT

use std::ptr;
use std::sync::Barrier;
use std::thread;

use asylum::Symbol;

#[test]
fn global_interner_reuses_static_storage_for_equal_text() {
    let (first, second) = "keywordkeyword".split_at("keyword".len());
    assert!(!ptr::eq(first, second));

    let symbol = asylum::intern_static(first);
    assert_eq!(symbol, asylum::intern_static(second));
    assert_eq!(symbol, asylum::intern(second));
    assert!(ptr::eq(symbol.as_str(), first));
    assert_ne!(symbol, asylum::intern(&first[..3]));
}

#[test]
fn global_interner_copies_dynamic_text_and_keeps_it_alive() {
    const TEXT: &str = "owned global text";
    let symbol = {
        let input = String::from(TEXT);
        let symbol = asylum::intern(&input);
        assert!(!ptr::eq(symbol.as_str(), input.as_str()));
        symbol
    };

    let text: &'static str = symbol.as_str();
    assert_eq!(text, TEXT);
    assert_eq!(symbol, asylum::intern_static(TEXT));
}

#[test]
fn global_symbols_and_their_text_can_leave_a_scope() {
    let global: Symbol<'static> = asylum::with(|interner| {
        let scoped = interner.intern("global across scopes");
        let global = asylum::intern(scoped.as_str());
        assert_eq!(scoped.as_str(), global.as_str());
        global
    });

    let text: &'static str = asylum::intern("global across scopes").as_str();
    assert_eq!(global.as_str(), text);
    assert_eq!(global, asylum::intern(text));
}

#[test]
fn global_interner_deduplicates_concurrent_inserts() {
    const TEXT: &str = "concurrent global text";
    let barrier = Barrier::new(2);

    thread::scope(|scope| {
        let other = scope.spawn(|| {
            barrier.wait();
            asylum::intern_static(TEXT)
        });

        let input = String::from(TEXT);
        barrier.wait();
        let symbol = asylum::intern(&input);
        assert_eq!(symbol, other.join().unwrap());
    });
}

#[test]
fn symbol_supports_as_ref_and_display() {
    let symbol = asylum::intern("display text");
    let text: &str = symbol.as_ref();

    assert_eq!(text, "display text");
    assert_eq!(symbol.to_string(), "display text");
    assert_eq!(format!("{symbol:>16}"), "    display text");
    assert_eq!(format!("{symbol:.7}"), "display");
    assert_eq!(format!("{symbol:*^11.7}"), "**display**");
}
