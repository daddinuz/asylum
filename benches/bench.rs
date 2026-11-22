use std::hint::black_box;
use std::sync::{Barrier, LazyLock};
use std::thread;
use std::time::{Duration, Instant};

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use ustr::Ustr;

const UNIQUE_WORDS: usize = 2048;
const DUPLICATE_WORDS: usize = 2048;
const DUPLICATE_VOCABULARY: usize = 32;
const CONTENTION_WORDS: usize = 512;
const CONTENTION_THREADS: usize = 4;

static BOUNDED_WORDS: LazyLock<Vec<String>> = LazyLock::new(|| generated_words(2048, 8, 64));
static LONG_WORDS: LazyLock<Vec<String>> = LazyLock::new(|| generated_words(512, 128, 256));

fn generated_words(count: usize, min_bytes: usize, max_bytes: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            let target_len = min_bytes + index % (max_bytes - min_bytes + 1);
            let mut word = format!("{index:04x}_");

            while word.len() < target_len {
                let byte = b'a' + ((index + word.len()) % 26) as u8;
                word.push(char::from(byte));
            }

            word
        })
        .collect()
}

fn intern_words<T>(words: &[&'static str], mut intern: impl FnMut(&'static str) -> T) -> Vec<T> {
    words.iter().map(|&word| intern(black_box(word))).collect()
}

fn bench_cold_from_empty(c: &mut Criterion, workload: &str, words: &[&'static str]) {
    let mut group = c.benchmark_group(format!("cold_from_empty/{workload}"));
    group.throughput(Throughput::Elements(words.len() as u64));

    group.bench_function("asylum_scoped", |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                elapsed += asylum::with(|interner| {
                    let start = Instant::now();
                    let symbols = intern_words(words, |word| interner.intern(word));
                    let elapsed = start.elapsed();
                    black_box(symbols);
                    elapsed
                });
            }
            elapsed
        });
    });

    group.bench_function("asylum_previous", |b| {
        b.iter_batched(
            asylum_previous::shrink_to_fit,
            |_| intern_words(words, asylum_previous::intern),
            BatchSize::PerIteration,
        );
    });
    asylum_previous::shrink_to_fit();

    group.bench_function("ustr", |b| {
        b.iter_batched(
            || {
                // SAFETY: Each iteration drops its symbols before the next reset.
                unsafe { ustr::_clear_cache() }
            },
            |_| intern_words(words, Ustr::from),
            BatchSize::PerIteration,
        );
    });

    group.bench_function("string_alloc", |b| {
        b.iter_batched(
            || (),
            |_| intern_words(words, str::to_owned),
            BatchSize::PerIteration,
        );
    });
}

fn bench_hot_lookup(c: &mut Criterion, workload: &str, words: &[&'static str]) {
    let mut group = c.benchmark_group(format!("hot_lookup/{workload}"));
    group.throughput(Throughput::Elements(words.len() as u64));

    asylum::with(|interner| {
        intern_words(words, |word| interner.intern(word));
        group.bench_function("asylum_scoped", |b| {
            b.iter(|| {
                let symbols = intern_words(words, |word| interner.intern(word));
                black_box(&symbols);
            });
        });
    });

    intern_words(words, asylum::intern_static);
    group.bench_function("asylum_global", |b| {
        b.iter(|| {
            let symbols = intern_words(words, asylum::intern);
            black_box(&symbols);
        });
    });

    let guards = intern_words(words, asylum_previous::intern);
    group.bench_function("asylum_previous", |b| {
        b.iter(|| {
            let symbols = intern_words(words, asylum_previous::intern);
            black_box(&symbols);
        });
    });
    drop(guards);
    asylum_previous::shrink_to_fit();

    intern_words(words, Ustr::from);
    group.bench_function("ustr", |b| {
        b.iter(|| {
            let symbols = intern_words(words, Ustr::from);
            black_box(&symbols);
        });
    });

    group.bench_function("string_alloc", |b| {
        b.iter(|| {
            let strings = intern_words(words, str::to_owned);
            black_box(&strings);
        });
    });
}

fn bench_cleanup(c: &mut Criterion, workload: &str, words: &[&'static str]) {
    let mut group = c.benchmark_group(format!("cleanup_drop/{workload}"));
    group.throughput(Throughput::Elements(words.len() as u64));

    group.bench_function("asylum_scoped", |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let start = asylum::with(|interner| {
                    let symbols = intern_words(words, |word| interner.intern(word));
                    let start = Instant::now();
                    drop(black_box(symbols));
                    start
                });
                // The lookup tables and arena have both been dropped.
                elapsed += start.elapsed();
            }
            elapsed
        });
    });

    group.bench_function("asylum_previous", |b| {
        b.iter_batched(
            || intern_words(words, asylum_previous::intern),
            |symbols| {
                drop(black_box(symbols));
                asylum_previous::shrink_to_fit();
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function("string_drop", |b| {
        b.iter_batched(
            || intern_words(words, str::to_owned),
            |strings| drop(black_box(strings)),
            BatchSize::PerIteration,
        );
    });
}

fn bench_hot_contention(c: &mut Criterion, words: &[&'static str]) {
    let words = &words[..CONTENTION_WORDS];
    let mut group = c.benchmark_group("hot_contention");
    group.throughput(Throughput::Elements(
        (words.len() * CONTENTION_THREADS) as u64,
    ));

    asylum::with(|interner| {
        intern_words(words, |word| interner.intern(word));
        group.bench_function("asylum_scoped", |b| {
            b.iter_custom(|iters| time_parallel(iters, words, |word| interner.intern(word)));
        });
    });

    intern_words(words, asylum::intern_static);
    group.bench_function("asylum_global", |b| {
        b.iter_custom(|iters| time_parallel(iters, words, asylum::intern));
    });

    let guards = intern_words(words, asylum_previous::intern);
    group.bench_function("asylum_previous", |b| {
        b.iter_custom(|iters| time_parallel(iters, words, asylum_previous::intern));
    });
    drop(guards);
    asylum_previous::shrink_to_fit();

    intern_words(words, Ustr::from);
    group.bench_function("ustr", |b| {
        b.iter_custom(|iters| time_parallel(iters, words, Ustr::from));
    });

    group.bench_function("string_alloc", |b| {
        b.iter_custom(|iters| time_parallel(iters, words, str::to_owned));
    });
}

fn time_parallel<T>(
    iters: u64,
    words: &[&'static str],
    intern: impl Fn(&'static str) -> T + Sync,
) -> Duration {
    let ready = Barrier::new(CONTENTION_THREADS + 1);
    let start = Barrier::new(CONTENTION_THREADS + 1);

    thread::scope(|scope| {
        let workers: Vec<_> = (0..CONTENTION_THREADS)
            .map(|_| {
                scope.spawn(|| {
                    ready.wait();
                    start.wait();

                    for _ in 0..iters {
                        let symbols = intern_words(words, &intern);
                        black_box(&symbols);
                    }
                })
            })
            .collect();

        ready.wait();
        let started_at = Instant::now();
        start.wait();

        for worker in workers {
            worker.join().unwrap();
        }

        started_at.elapsed()
    })
}

fn criterion_benchmark(c: &mut Criterion) {
    let unique: Vec<_> = include_str!("english.txt")
        .lines()
        .take(UNIQUE_WORDS)
        .collect();
    let duplicate_heavy: Vec<_> = (0..DUPLICATE_WORDS)
        .map(|index| unique[index % DUPLICATE_VOCABULARY])
        .collect();
    let bounded: Vec<_> = BOUNDED_WORDS.iter().map(String::as_str).collect();
    let long: Vec<_> = LONG_WORDS.iter().map(String::as_str).collect();

    for (workload, words) in [
        ("short_2048", unique.as_slice()),
        ("duplicate_heavy_2048_from_32", &duplicate_heavy),
        ("bounded_64_2048", &bounded),
        ("long_stress_128_256_512", &long),
    ] {
        bench_cold_from_empty(c, workload, words);
        bench_hot_lookup(c, workload, words);
        bench_cleanup(c, workload, words);
    }

    bench_hot_contention(c, &unique);
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
