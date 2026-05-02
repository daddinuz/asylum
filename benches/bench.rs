use std::hint::black_box;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ustr::Ustr;

const UNIQUE_WORDS: usize = 2048;
const DUPLICATE_WORDS: usize = 2048;
const DUPLICATE_VOCABULARY: usize = 32;
const BOUNDED_WORDS: usize = 2048;
const BOUNDED_MAX_BYTES: usize = 64;
const LONG_STRESS_WORDS: usize = 512;
const LONG_STRESS_MIN_BYTES: usize = 128;
const LONG_STRESS_MAX_BYTES: usize = 256;
const CONTENTION_WORDS: usize = 512;
const CONTENTION_THREADS: usize = 4;

fn load_words() -> Vec<String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&manifest_dir).join("benches").join("english.txt");

    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .take(UNIQUE_WORDS)
        .map(String::from)
        .collect()
}

fn duplicate_heavy_words(words: &[String]) -> Vec<String> {
    (0..DUPLICATE_WORDS)
        .map(|index| words[index % DUPLICATE_VOCABULARY].clone())
        .collect()
}

fn bounded_64_words() -> Vec<String> {
    (0..BOUNDED_WORDS)
        .map(|index| {
            let target_len = 8 + index % (BOUNDED_MAX_BYTES - 8 + 1);
            generated_word(index, target_len)
        })
        .collect()
}

fn long_stress_words() -> Vec<String> {
    (0..LONG_STRESS_WORDS)
        .map(|index| {
            let target_len =
                LONG_STRESS_MIN_BYTES + index % (LONG_STRESS_MAX_BYTES - LONG_STRESS_MIN_BYTES + 1);
            generated_word(index, target_len)
        })
        .collect()
}

fn generated_word(index: usize, target_len: usize) -> String {
    let mut word = format!("{index:04x}_");

    while word.len() < target_len {
        let byte = b'a' + ((index + word.len()) % 26) as u8;
        word.push(char::from(byte));
    }

    word
}

fn collect_asylum(words: &[String]) -> Vec<asylum::Symbol> {
    words
        .iter()
        .map(|word| asylum::intern(black_box(word.as_str())))
        .collect()
}

fn collect_asylum_previous(words: &[String]) -> Vec<asylum_previous::Symbol> {
    words
        .iter()
        .map(|word| asylum_previous::intern(black_box(word.as_str())))
        .collect()
}

fn collect_ustr(words: &[String]) -> Vec<Ustr> {
    words
        .iter()
        .map(|word| Ustr::from(black_box(word.as_str())))
        .collect()
}

fn collect_string(words: &[String]) -> Vec<String> {
    words
        .iter()
        .map(|word| black_box(word.as_str()).to_string())
        .collect()
}

fn bench_transient_reuse_capacity(c: &mut Criterion, workload_name: &str, words: &[String]) {
    let mut group = c.benchmark_group(format!("transient_reuse_capacity/{workload_name}"));
    group.throughput(Throughput::Elements(words.len() as u64));

    group.bench_function(BenchmarkId::new("asylum_current", words.len()), |b| {
        b.iter_batched(
            || {
                asylum::collect_unused();
                assert_eq!(asylum::size(), 0);
            },
            |_| {
                let symbols = collect_asylum(words);
                black_box(&symbols);
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function(BenchmarkId::new("asylum_previous", words.len()), |b| {
        b.iter_batched(
            || {
                assert_eq!(asylum_previous::size(), 0);
            },
            |_| {
                let symbols = collect_asylum_previous(words);
                black_box(&symbols);
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function(BenchmarkId::new("ustr", words.len()), |b| {
        b.iter_batched(
            || unsafe {
                ustr::_clear_cache();
            },
            |_| {
                let symbols = collect_ustr(words);
                black_box(&symbols);
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function(BenchmarkId::new("string_alloc", words.len()), |b| {
        b.iter(|| {
            let strings = collect_string(words);
            black_box(&strings);
        });
    });

    group.finish();

    asylum::shrink_to_fit();
    assert_eq!(asylum::size(), 0);
    assert_eq!(asylum_previous::size(), 0);
}

fn bench_cold_from_empty(c: &mut Criterion, workload_name: &str, words: &[String]) {
    let mut group = c.benchmark_group(format!("cold_from_empty/{workload_name}"));
    group.throughput(Throughput::Elements(words.len() as u64));

    group.bench_function(BenchmarkId::new("asylum_current", words.len()), |b| {
        b.iter_batched(
            || {
                asylum::shrink_to_fit();
                assert_eq!(asylum::size(), 0);
            },
            |_| {
                let symbols = collect_asylum(words);
                black_box(&symbols);
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function(BenchmarkId::new("asylum_previous", words.len()), |b| {
        b.iter_batched(
            || {
                assert_eq!(asylum_previous::size(), 0);
                asylum_previous::shrink_to_fit();
            },
            |_| {
                let symbols = collect_asylum_previous(words);
                black_box(&symbols);
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function(BenchmarkId::new("ustr", words.len()), |b| {
        b.iter_batched(
            || unsafe {
                ustr::_clear_cache();
            },
            |_| {
                let symbols = collect_ustr(words);
                black_box(&symbols);
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function(BenchmarkId::new("string_alloc", words.len()), |b| {
        b.iter(|| {
            let strings = collect_string(words);
            black_box(&strings);
        });
    });

    group.finish();

    asylum::shrink_to_fit();
    assert_eq!(asylum::size(), 0);
    assert_eq!(asylum_previous::size(), 0);
}

fn bench_hot_lookup(c: &mut Criterion, workload_name: &str, words: &[String]) {
    let mut group = c.benchmark_group(format!("hot_lookup/{workload_name}"));
    group.throughput(Throughput::Elements(words.len() as u64));

    asylum::shrink_to_fit();
    let asylum_guards = collect_asylum(words);
    assert!(asylum::size() > 0);

    group.bench_function(BenchmarkId::new("asylum_current", words.len()), |b| {
        b.iter(|| {
            let symbols = collect_asylum(words);
            black_box(&symbols);
        });
    });

    drop(asylum_guards);
    asylum::shrink_to_fit();
    assert_eq!(asylum::size(), 0);

    asylum_previous::shrink_to_fit();
    let asylum_previous_guards = collect_asylum_previous(words);
    assert!(asylum_previous::size() > 0);

    group.bench_function(BenchmarkId::new("asylum_previous", words.len()), |b| {
        b.iter(|| {
            let symbols = collect_asylum_previous(words);
            black_box(&symbols);
        });
    });

    drop(asylum_previous_guards);
    assert_eq!(asylum_previous::size(), 0);

    unsafe {
        ustr::_clear_cache();
    }
    let ustr_guards = collect_ustr(words);

    group.bench_function(BenchmarkId::new("ustr", words.len()), |b| {
        b.iter(|| {
            let symbols = collect_ustr(words);
            black_box(&symbols);
        });
    });

    drop(ustr_guards);
    unsafe {
        ustr::_clear_cache();
    }

    group.bench_function(BenchmarkId::new("string_alloc", words.len()), |b| {
        b.iter(|| {
            let strings = collect_string(words);
            black_box(&strings);
        });
    });

    group.finish();
}

fn bench_cleanup(c: &mut Criterion, workload_name: &str, words: &[String]) {
    let mut group = c.benchmark_group(format!("cleanup_drop/{workload_name}"));
    group.throughput(Throughput::Elements(words.len() as u64));

    group.bench_function(
        BenchmarkId::new("asylum_current_drop_last_refs", words.len()),
        |b| {
            b.iter_batched(
                || {
                    asylum::shrink_to_fit();
                    assert_eq!(asylum::size(), 0);
                    collect_asylum(words)
                },
                |symbols| {
                    drop(black_box(symbols));
                },
                BatchSize::PerIteration,
            );
        },
    );

    group.bench_function(
        BenchmarkId::new("asylum_previous_drop_last_refs", words.len()),
        |b| {
            b.iter_batched(
                || {
                    assert_eq!(asylum_previous::size(), 0);
                    collect_asylum_previous(words)
                },
                |symbols| {
                    drop(black_box(symbols));
                    assert_eq!(asylum_previous::size(), 0);
                },
                BatchSize::PerIteration,
            );
        },
    );

    group.bench_function(BenchmarkId::new("string_drop", words.len()), |b| {
        b.iter_batched(
            || collect_string(words),
            |strings| {
                drop(black_box(strings));
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();
    asylum::shrink_to_fit();
    assert_eq!(asylum::size(), 0);
}

fn bench_hot_contention(c: &mut Criterion, words: &[String]) {
    let words: Arc<[String]> = words.iter().take(CONTENTION_WORDS).cloned().collect();
    let mut group = c.benchmark_group("hot_contention");
    group.throughput(Throughput::Elements(
        (words.len() * CONTENTION_THREADS) as u64,
    ));

    asylum::shrink_to_fit();
    let asylum_guards = collect_asylum(&words);
    group.bench_function(
        BenchmarkId::new("asylum_current", format!("{CONTENTION_THREADS}_threads")),
        |b| {
            b.iter_custom(|iters| time_parallel(iters, words.clone(), asylum::intern));
        },
    );
    drop(asylum_guards);
    asylum::shrink_to_fit();
    assert_eq!(asylum::size(), 0);

    asylum_previous::shrink_to_fit();
    let asylum_previous_guards = collect_asylum_previous(&words);
    group.bench_function(
        BenchmarkId::new("asylum_previous", format!("{CONTENTION_THREADS}_threads")),
        |b| {
            b.iter_custom(|iters| time_parallel(iters, words.clone(), asylum_previous::intern));
        },
    );
    drop(asylum_previous_guards);
    asylum_previous::shrink_to_fit();
    assert_eq!(asylum_previous::size(), 0);

    unsafe {
        ustr::_clear_cache();
    }
    let ustr_guards = collect_ustr(&words);
    group.bench_function(
        BenchmarkId::new("ustr", format!("{CONTENTION_THREADS}_threads")),
        |b| {
            b.iter_custom(|iters| time_parallel(iters, words.clone(), Ustr::from));
        },
    );
    drop(ustr_guards);
    unsafe {
        ustr::_clear_cache();
    }

    group.bench_function(
        BenchmarkId::new("string_alloc", format!("{CONTENTION_THREADS}_threads")),
        |b| {
            b.iter_custom(|iters| time_parallel(iters, words.clone(), |word| word.to_string()));
        },
    );

    group.finish();
}

fn time_parallel<T>(iters: u64, words: Arc<[String]>, intern: fn(&str) -> T) -> Duration
where
    T: Send + 'static,
{
    let ready = Arc::new(Barrier::new(CONTENTION_THREADS + 1));
    let start = Arc::new(Barrier::new(CONTENTION_THREADS + 1));

    let handles = (0..CONTENTION_THREADS)
        .map(|_| {
            let ready = ready.clone();
            let start = start.clone();
            let words = words.clone();

            thread::spawn(move || {
                ready.wait();
                start.wait();

                for _ in 0..iters {
                    let symbols = words
                        .iter()
                        .map(|word| intern(black_box(word.as_str())))
                        .collect::<Vec<_>>();
                    black_box(&symbols);
                }
            })
        })
        .collect::<Vec<_>>();

    ready.wait();
    let started_at = Instant::now();
    start.wait();

    for handle in handles {
        handle.join().unwrap();
    }

    started_at.elapsed()
}

fn criterion_benchmark(c: &mut Criterion) {
    let unique = load_words();
    let duplicate_heavy = duplicate_heavy_words(&unique);
    let bounded_64 = bounded_64_words();
    let long_stress = long_stress_words();

    bench_transient_reuse_capacity(c, "short_2048", &unique);
    bench_transient_reuse_capacity(c, "duplicate_heavy_2048_from_32", &duplicate_heavy);
    bench_transient_reuse_capacity(c, "bounded_64_2048", &bounded_64);
    bench_transient_reuse_capacity(c, "long_stress_128_256_512", &long_stress);

    bench_cold_from_empty(c, "short_2048", &unique);
    bench_cold_from_empty(c, "duplicate_heavy_2048_from_32", &duplicate_heavy);
    bench_cold_from_empty(c, "bounded_64_2048", &bounded_64);
    bench_cold_from_empty(c, "long_stress_128_256_512", &long_stress);

    bench_hot_lookup(c, "short_2048", &unique);
    bench_hot_lookup(c, "duplicate_heavy_2048_from_32", &duplicate_heavy);
    bench_hot_lookup(c, "bounded_64_2048", &bounded_64);

    bench_cleanup(c, "short_2048", &unique);
    bench_cleanup(c, "duplicate_heavy_2048_from_32", &duplicate_heavy);
    bench_cleanup(c, "bounded_64_2048", &bounded_64);

    bench_hot_contention(c, &unique);
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
