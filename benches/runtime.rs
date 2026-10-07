mod common;

use std::fmt;

use common::{chase, reduce, Dataset, Dictionary, Offset, Packed, Rng, Size, Subject, LENS, RANDOM_READS, READ_LENS};
use divan::counter::{BytesCount, ItemsCount};
use divan::{black_box, AllocProfiler, Bencher};
use shrewd_rs::Shrewd;

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
}

// ---------------------------------------------------------------------------------------------
// Encoding and serialization (Shrewd only: a Vec<i64> has nothing equivalent to measure)
// ---------------------------------------------------------------------------------------------

#[divan::bench(types = [Size, Offset, Dictionary], args = LENS)]
fn pack<D: Dataset>(bencher: Bencher, len: usize) {
    let data = D::generate(len);
    common::packed::<D>(len);
    bencher
        .counter(ItemsCount::new(len))
        .with_inputs(|| data.clone())
        .bench_values(Shrewd::pack);
}

#[divan::bench(types = [Size, Offset, Dictionary], args = READ_LENS)]
fn to_bytes<D: Dataset>(bencher: Bencher, len: usize) {
    let packed = common::packed::<D>(len);
    bencher
        .counter(BytesCount::new(packed.to_bytes().len()))
        .bench(|| black_box(&packed).to_bytes());
}

#[divan::bench(types = [Size, Offset, Dictionary], args = READ_LENS)]
fn from_bytes<D: Dataset>(bencher: Bencher, len: usize) {
    let bytes = common::packed::<D>(len).to_bytes();
    bencher
        .counter(BytesCount::new(bytes.len()))
        .bench(|| Shrewd::from_bytes(black_box(&bytes)).unwrap());
}

// ---------------------------------------------------------------------------------------------
// Vec<i64> vs Shrewd: the same operations, compared two ways
// ---------------------------------------------------------------------------------------------

/// A memory budget each container is filled to, labelled with the cache level it targets.
/// Sized below common cache sizes (L1d >= 32 KiB, L2 >= 512 KiB, L3 >= 16 MiB) so each
/// container stays in that level on most desktop and server CPUs.
#[derive(Clone, Copy)]
struct Budget {
    level: &'static str,
    bytes: usize,
}

impl fmt::Display for Budget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.bytes {
            b if b >= 1 << 20 => write!(f, "{} {} MiB", self.level, b >> 20),
            b => write!(f, "{} {} KiB", self.level, b >> 10),
        }
    }
}

const BUDGETS: [Budget; 4] = [
    Budget { level: "L1", bytes: 32 << 10 },
    Budget { level: "L2", bytes: 512 << 10 },
    Budget { level: "L3", bytes: 16 << 20 },
    Budget { level: "RAM", bytes: 128 << 20 },
];

/// Equal elements: every container holds `len` values, so each lands wherever its size puts it.
fn by_elements<S: Subject>(len: usize) -> S {
    S::build(len)
}

/// Equal memory: every container is filled to the budget, so each holds as many values as fit.
fn by_memory<S: Subject>(budget: Budget) -> S {
    let subject = S::build(budget.bytes / S::BYTES_PER_ELEMENT);
    // Fixed overhead (struct, dictionary table) may go slightly over; anything more is a sizing bug.
    assert!(subject.footprint() <= budget.bytes + 4096, "container exceeds the {budget} budget");
    subject
}

/// Generates one module of comparison benchmarks. `$setup` builds a container from `$arg`.
macro_rules! comparisons {
    ($module:ident, $arg:ty, $args:expr, $setup:ident) => {
        mod $module {
            use super::*;

            /// Independent random reads: several can be in flight at once (read throughput).
            #[divan::bench(types = [Vec<i64>, Packed<Size>, Packed<Offset>, Packed<Dictionary>], args = $args)]
            fn get_random<S: Subject>(bencher: Bencher, arg: $arg) {
                let subject = $setup::<S>(arg);
                let len = subject.len();
                let mut rng = Rng::new(0x1D1CE5);
                bencher.counter(ItemsCount::new(RANDOM_READS)).bench_local(|| {
                    let subject = black_box(&subject);
                    (0..RANDOM_READS).fold(0i64, |sum, _| {
                        sum.wrapping_add(subject.get(reduce(rng.next_u64(), len)).unwrap())
                    })
                });
            }

            /// Dependent random reads: each index needs the previous value (single-lookup latency).
            #[divan::bench(types = [Vec<i64>, Packed<Size>, Packed<Offset>, Packed<Dictionary>], args = $args)]
            fn get_latency<S: Subject>(bencher: Bencher, arg: $arg) {
                let subject = $setup::<S>(arg);
                let len = subject.len();
                let mut state = 0x1D1CE5u64;
                let mut value = 0i64;
                bencher.counter(ItemsCount::new(RANDOM_READS)).bench_local(|| {
                    let subject = black_box(&subject);
                    for _ in 0..RANDOM_READS {
                        value = subject.get(chase(&mut state, value, len)).unwrap();
                    }
                    value
                });
            }

            /// `get(i)` for every index in order.
            #[divan::bench(types = [Vec<i64>, Packed<Size>, Packed<Offset>, Packed<Dictionary>], args = $args)]
            fn get_loop<S: Subject>(bencher: Bencher, arg: $arg) {
                let subject = $setup::<S>(arg);
                bencher.counter(ItemsCount::new(subject.len())).bench(|| {
                    let subject = black_box(&subject);
                    (0..subject.len()).fold(0i64, |sum, index| sum.wrapping_add(subject.get(index).unwrap()))
                });
            }

            /// A plain `for` loop over all values.
            #[divan::bench(types = [Vec<i64>, Packed<Size>, Packed<Offset>, Packed<Dictionary>], args = $args)]
            fn for_loop<S: Subject>(bencher: Bencher, arg: $arg) {
                let subject = $setup::<S>(arg);
                bencher.counter(ItemsCount::new(subject.len())).bench(|| black_box(&subject).sum_for_loop());
            }

            /// `iter().fold` over all values (Shrewd: default 64-value blocks).
            #[divan::bench(types = [Vec<i64>, Packed<Size>, Packed<Offset>, Packed<Dictionary>], args = $args)]
            fn fold<S: Subject>(bencher: Bencher, arg: $arg) {
                let subject = $setup::<S>(arg);
                bencher.counter(ItemsCount::new(subject.len())).bench(|| black_box(&subject).sum_fold());
            }

            /// Shrewd: `iter_buffered::<1024>().fold`. Vec: same as `fold`.
            #[divan::bench(types = [Vec<i64>, Packed<Size>, Packed<Offset>, Packed<Dictionary>], args = $args)]
            fn fold_buffered<S: Subject>(bencher: Bencher, arg: $arg) {
                let subject = $setup::<S>(arg);
                bencher.counter(ItemsCount::new(subject.len())).bench(|| black_box(&subject).sum_fold_buffered());
            }

            /// Every value copied out in 1024-value chunks (Shrewd: `decode_into`; Vec: `copy_from_slice`).
            #[divan::bench(types = [Vec<i64>, Packed<Size>, Packed<Offset>, Packed<Dictionary>], args = $args)]
            fn copy_chunks<S: Subject>(bencher: Bencher, arg: $arg) {
                let subject = $setup::<S>(arg);
                let mut chunk = [0i64; 1024];
                bencher.counter(ItemsCount::new(subject.len())).bench_local(|| {
                    let subject = black_box(&subject);
                    let mut start = 0;
                    loop {
                        let n = subject.copy_into(start, &mut chunk);
                        if n == 0 {
                            break;
                        }
                        black_box(&chunk);
                        start += n;
                    }
                });
            }

            /// All values into a new `Vec<i64>` with `collect`.
            #[divan::bench(types = [Vec<i64>, Packed<Size>, Packed<Offset>, Packed<Dictionary>], args = $args)]
            fn collect<S: Subject>(bencher: Bencher, arg: $arg) {
                let subject = $setup::<S>(arg);
                bencher.counter(ItemsCount::new(subject.len())).bench(|| black_box(&subject).collect());
            }
        }
    };
}

comparisons!(equal_elements, usize, READ_LENS, by_elements);
comparisons!(equal_memory, Budget, BUDGETS, by_memory);
