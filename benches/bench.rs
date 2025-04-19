#[macro_use]
extern crate criterion;

use std::fmt::Debug;
use std::hash::Hash;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use criterion::Criterion;

use ustr::Ustr;

fn no_contention<T>(
    criterion: &mut Criterion,
    id: &str,
    words: &[String],
    intern: fn(&str) -> T,
    cleanup: fn(),
) where
    T: AsRef<str> + Eq + Hash,
{
    criterion.bench_function(id, move |bencher| {
        bencher.iter(|| {
            cleanup();
            let sut = words.iter().map(|s| intern(s)).collect::<Vec<_>>();
            assert_eq!(sut.len(), words.len());
            assert!(sut.iter().map(AsRef::as_ref).eq(words));
        });
    });
}

fn soft_contention<T>(
    criterion: &mut Criterion,
    id: &str,
    words: Arc<[String]>,
    intern: fn(&str) -> T,
    cleanup: fn(),
) where
    T: 'static + AsRef<str> + Debug + Eq + Hash + Send + Sync,
{
    criterion.bench_function(id, move |bencher| {
        bencher.iter(|| {
            cleanup();

            let t1 = {
                let words = words.clone();
                thread::spawn(move || words.iter().rev().map(|s| intern(s)).collect::<Vec<_>>())
            };

            let sut2 = words.iter().map(|s| intern(s)).collect::<Vec<_>>();
            let sut1 = t1.join().unwrap();

            assert_eq!(sut1.len(), words.len());
            assert_eq!(sut2.len(), words.len());
            assert!(sut1.iter().rev().eq(sut2.iter()));
            assert!(sut2.iter().map(AsRef::as_ref).eq(words.iter()));
        });
    });
}

fn hard_contention<T>(
    criterion: &mut Criterion,
    id: &str,
    words: Arc<[String]>,
    intern: fn(&str) -> T,
    cleanup: fn(),
) where
    T: 'static + AsRef<str> + Debug + Eq + Hash + Send + Sync,
{
    criterion.bench_function(id, move |bencher| {
        bencher.iter(|| {
            cleanup();

            let t1 = {
                let words = words.clone();
                thread::spawn(move || words.iter().map(|s| intern(s)).collect::<Vec<_>>())
            };

            let t2 = {
                let words = words.clone();
                thread::spawn(move || words.iter().map(|s| intern(s)).collect::<Vec<_>>())
            };

            let t3 = {
                let words = words.clone();
                thread::spawn(move || words.iter().map(|s| intern(s)).collect::<Vec<_>>())
            };

            let sut4 = words.iter().map(|s| intern(s)).collect::<Vec<_>>();
            let sut3 = t3.join().unwrap();
            let sut2 = t2.join().unwrap();
            let sut1 = t1.join().unwrap();

            assert_eq!(sut1.len(), words.len());
            assert_eq!(sut2.len(), words.len());
            assert_eq!(sut3.len(), words.len());
            assert_eq!(sut4.len(), words.len());
            assert_eq!(sut1, sut2);
            assert_eq!(sut2, sut3);
            assert_eq!(sut3, sut4);
            assert!(sut4.iter().map(AsRef::as_ref).eq(words.iter()));
        });
    });
}

fn criterion_benchmark(criterion: &mut Criterion) {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&manifest_dir).join("benches").join("english.txt");

    let wordlist = std::fs::read_to_string(path).unwrap();
    let words: Arc<[String]> = wordlist
        .lines()
        .map(String::from)
        .collect::<Vec<_>>()
        .into();

    let noop = || {};
    let ustr_cleanup = || unsafe { ustr::_clear_cache() };
    let asylum_cleanup = || assert_eq!(asylum::size(), 0);

    no_contention(
        criterion,
        "asylum::intern (no contention)",
        &words,
        asylum::intern,
        asylum_cleanup,
    );
    no_contention(
        criterion,
        "Ustr::from (no contention)",
        &words,
        Ustr::from,
        ustr_cleanup,
    );
    no_contention(
        criterion,
        "str::to_string (no contention)",
        &words,
        str::to_string,
        noop,
    );

    soft_contention(
        criterion,
        "asylum::intern (soft contention)",
        words.clone(),
        asylum::intern,
        asylum_cleanup,
    );
    soft_contention(
        criterion,
        "Ustr::from (soft contention)",
        words.clone(),
        Ustr::from,
        ustr_cleanup,
    );
    soft_contention(
        criterion,
        "str::to_string (soft contention)",
        words.clone(),
        str::to_string,
        noop,
    );

    hard_contention(
        criterion,
        "asylum::intern (hard contention)",
        words.clone(),
        asylum::intern,
        asylum_cleanup,
    );

    hard_contention(
        criterion,
        "Ustr::from (hard contention)",
        words.clone(),
        Ustr::from,
        ustr_cleanup,
    );
    hard_contention(
        criterion,
        "str::to_string (hard contention)",
        words.clone(),
        str::to_string,
        noop,
    );
}

criterion_group!(
    name = benches;
    config = Criterion::default();
    targets = criterion_benchmark
);

criterion_main!(benches);
