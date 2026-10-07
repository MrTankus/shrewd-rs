//! Deterministic datasets shared by the runtime and memory benchmarks.
//!
//! Only `Shrewd` is public, so each dataset is shaped to make the router pick one specific
//! strategy. `packed` asserts that choice, so a routing change fails loudly instead of
//! silently benchmarking a different compressor.

#![allow(dead_code)]

use std::mem::size_of;

use shrewd_rs::Shrewd;

/// Lengths chosen by where the *packed* data (1-2 bytes per element for these datasets) lands
/// in the cache hierarchy, sized for a desktop-class core (~48 KiB L1d, 1 MiB L2, 32 MiB L3):
/// 4Ki -> L1, 256Ki -> L2, 4Mi -> L3. `pack` reads 8 bytes per element of input, so its
/// working set is 8x larger than these labels suggest.
pub const LENS: [usize; 3] = [1 << 12, 1 << 18, 1 << 22];

/// `LENS` plus a length whose packed data (64-128 MiB) is past any L3, so reads come from DRAM.
/// Kept out of `pack`, where each sample would clone 512 MiB of input.
pub const READ_LENS: [usize; 4] = [1 << 12, 1 << 18, 1 << 22, 1 << 26];

/// Reads per sample in the random-access benchmarks: ~1 µs or more per sample, far above
/// timer resolution. Indices are generated on the fly and never repeat between samples, so
/// this count does not bound the set of cache lines touched.
pub const RANDOM_READS: usize = 1024;

/// splitmix64: fast, reproducible, and good enough to defeat branch prediction.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..bound`.
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

pub trait Dataset {
    const STRATEGY: &'static str;
    /// Bytes each element takes once packed, used to size the equal-memory benchmarks.
    const PACKED_BYTES_PER_ELEMENT: usize;

    fn value(rng: &mut Rng) -> i64;

    fn generate(len: usize) -> Vec<i64> {
        let mut rng = Rng::new(0x5EED ^ len as u64);
        (0..len).map(|_| Self::value(&mut rng)).collect()
    }
}

/// Values spread across the whole i16 range: no narrower offset or small dictionary exists.
pub struct Size;

impl Dataset for Size {
    const STRATEGY: &'static str = "Size";
    const PACKED_BYTES_PER_ELEMENT: usize = 2;

    fn value(rng: &mut Rng) -> i64 {
        rng.below(60_001) as i64 - 30_000
    }
}

/// A narrowband sitting far from zero: i64 values whose offsets fit in i8.
pub struct Offset;

impl Dataset for Offset {
    const STRATEGY: &'static str = "Offset";
    const PACKED_BYTES_PER_ELEMENT: usize = 1;

    fn value(rng: &mut Rng) -> i64 {
        1_000_000_000_000 + rng.below(251) as i64
    }
}

/// 16 widely spread i64 values: wide range, very low cardinality.
pub struct Dictionary;

impl Dataset for Dictionary {
    const STRATEGY: &'static str = "Dictionary";
    const PACKED_BYTES_PER_ELEMENT: usize = 1;

    fn value(rng: &mut Rng) -> i64 {
        (rng.below(16) as i64 - 8) * 1_000_000_000_000_000
    }
}

pub fn packed<D: Dataset>(len: usize) -> Shrewd {
    let packed = Shrewd::pack(D::generate(len));
    assert_eq!(packed.compressor_type().as_str(), D::STRATEGY, "dataset routed to the wrong strategy (len {len})");
    packed
}

/// Maps a well-mixed 64-bit value onto `0..len` with a multiply instead of a division
/// (Lemire's range reduction), so index generation stays cheap next to a cache-hit read.
#[inline(always)]
pub fn reduce(hash: u64, len: usize) -> usize {
    ((hash as u128 * len as u128) >> 64) as usize
}

/// Next index of a dependent read chain: it cannot be computed before `value`, the result of
/// the previous read, has arrived, so reads cannot overlap. One multiply of extra latency.
#[inline(always)]
pub fn chase(state: &mut u64, value: i64, len: usize) -> usize {
    *state = (*state ^ value as u64).wrapping_mul(0x9E3779B97F4A7C15) | 1;
    reduce(*state, len)
}

/// A container under comparison: a plain `Vec<i64>`, or a `Shrewd` packed from one dataset.
/// Each method is the container's natural way of doing the same job, so results compare directly.
pub trait Subject: Sync {
    const BYTES_PER_ELEMENT: usize;

    fn build(len: usize) -> Self;
    fn len(&self) -> usize;
    /// Heap plus the value itself.
    fn footprint(&self) -> usize;
    fn get(&self, index: usize) -> Option<i64>;
    fn sum_for_loop(&self) -> i64;
    fn sum_fold(&self) -> i64;
    /// `Shrewd`: `iter_buffered::<1024>().fold`. `Vec`: the same as `sum_fold`.
    fn sum_fold_buffered(&self) -> i64;
    /// Copies up to `out.len()` values starting at `start`; returns how many.
    fn copy_into(&self, start: usize, out: &mut [i64]) -> usize;
    fn collect(&self) -> Vec<i64>;
}

impl Subject for Vec<i64> {
    const BYTES_PER_ELEMENT: usize = size_of::<i64>();

    fn build(len: usize) -> Self {
        Size::generate(len)
    }

    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn footprint(&self) -> usize {
        self.capacity() * size_of::<i64>() + size_of::<Vec<i64>>()
    }

    #[inline]
    fn get(&self, index: usize) -> Option<i64> {
        self.as_slice().get(index).copied()
    }

    fn sum_for_loop(&self) -> i64 {
        let mut sum = 0i64;
        for &value in self {
            sum = sum.wrapping_add(value);
        }
        sum
    }

    fn sum_fold(&self) -> i64 {
        self.iter().fold(0i64, |sum, &value| sum.wrapping_add(value))
    }

    fn sum_fold_buffered(&self) -> i64 {
        self.sum_fold()
    }

    fn copy_into(&self, start: usize, out: &mut [i64]) -> usize {
        let n = out.len().min(self.as_slice().len().saturating_sub(start));
        out[..n].copy_from_slice(&self[start..start + n]);
        n
    }

    fn collect(&self) -> Vec<i64> {
        self.iter().copied().collect()
    }
}

/// A `Shrewd` packed from dataset `D` (the router's choice is asserted when it is built).
pub struct Packed<D>(Shrewd, std::marker::PhantomData<fn() -> D>);

impl<D: Dataset> Subject for Packed<D> {
    const BYTES_PER_ELEMENT: usize = D::PACKED_BYTES_PER_ELEMENT;

    fn build(len: usize) -> Self {
        Packed(packed::<D>(len), std::marker::PhantomData)
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn footprint(&self) -> usize {
        self.0.memory_size()
    }

    #[inline]
    fn get(&self, index: usize) -> Option<i64> {
        self.0.get(index)
    }

    fn sum_for_loop(&self) -> i64 {
        let mut sum = 0i64;
        for value in &self.0 {
            sum = sum.wrapping_add(value);
        }
        sum
    }

    fn sum_fold(&self) -> i64 {
        self.0.iter().fold(0i64, |sum, value| sum.wrapping_add(value))
    }

    fn sum_fold_buffered(&self) -> i64 {
        self.0.iter_buffered::<1024>().fold(0i64, |sum, value| sum.wrapping_add(value))
    }

    fn copy_into(&self, start: usize, out: &mut [i64]) -> usize {
        self.0.decode_into(start, out)
    }

    fn collect(&self) -> Vec<i64> {
        self.0.iter().collect()
    }
}
