mod common;

use common::{chase, reduce, Dataset, Dictionary, Offset, Rng, Size, LENS, RANDOM_READS, READ_LENS};
use divan::counter::{BytesCount, ItemsCount};
use divan::{black_box, AllocProfiler, Bencher};
use shrewd_rs::shrewd::{Compressor, Shrewd};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
}

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
fn get_random<D: Dataset>(bencher: Bencher, len: usize) {
    let packed = common::packed::<D>(len);
    let mut rng = Rng::new(0x1D1CE5);
    bencher.counter(ItemsCount::new(RANDOM_READS)).bench_local(|| {
        let packed = black_box(&packed);
        (0..RANDOM_READS).fold(0i64, |sum, _| {
            sum.wrapping_add(packed.get(reduce(rng.next_u64(), len)).unwrap())
        })
    });
}

#[divan::bench(types = [Size, Offset, Dictionary], args = READ_LENS)]
fn get_latency<D: Dataset>(bencher: Bencher, len: usize) {
    let packed = common::packed::<D>(len);
    let mut state = 0x1D1CE5u64;
    let mut value = 0i64;
    bencher.counter(ItemsCount::new(RANDOM_READS)).bench_local(|| {
        let packed = black_box(&packed);
        for _ in 0..RANDOM_READS {
            value = packed.get(chase(&mut state, value, len)).unwrap();
        }
        value
    });
}

#[divan::bench(types = [Size, Offset, Dictionary], args = READ_LENS)]
fn get_sequential<D: Dataset>(bencher: Bencher, len: usize) {
    let packed = common::packed::<D>(len);
    bencher.counter(ItemsCount::new(len)).bench(|| {
        let packed = black_box(&packed);
        (0..packed.length()).fold(0i64, |sum, index| sum.wrapping_add(packed.get(index).unwrap()))
    });
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

mod overhead {
    use super::*;

    const LEN: usize = 1 << 26;

    #[divan::bench]
    fn random_indices(bencher: Bencher) {
        let mut rng = Rng::new(0x1D1CE5);
        bencher.counter(ItemsCount::new(RANDOM_READS)).bench_local(|| {
            (0..RANDOM_READS).fold(0usize, |sum, _| sum.wrapping_add(reduce(rng.next_u64(), black_box(LEN))))
        });
    }

    #[divan::bench]
    fn latency_chain(bencher: Bencher) {
        let mut state = 0x1D1CE5u64;
        let mut value = 0i64;
        bencher.counter(ItemsCount::new(RANDOM_READS)).bench_local(|| {
            for _ in 0..RANDOM_READS {
                value = black_box(chase(&mut state, value, black_box(LEN)) as i64);
            }
            value
        });
    }
}
