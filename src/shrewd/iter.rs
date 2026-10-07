//! The iterator returned by [`Shrewd::iter`] and [`Shrewd::iter_buffered`].

use super::Shrewd;

/// The number of values that [`Shrewd::iter`] decodes at a time.
///
/// The buffer takes 512 bytes, which is small enough for the limited stacks of embedded
/// systems.
pub const DEFAULT_BLOCK_SIZE: usize = 64;

/// An iterator over the values of a compressed vector.
///
/// It decodes `BLOCK` values at a time into a buffer that is part of the iterator, and returns
/// them one by one. Create it with [`Shrewd::iter`] or [`Shrewd::iter_buffered`].
///
/// The iterator knows exactly how many values are left, so `len()` works and `collect()`
/// allocates once. Methods built on `fold`, such as `sum` and `for_each`, decode whole blocks
/// at a time and are faster than a `for` loop.
#[derive(Debug, Clone)]
pub struct Iter<'a, const BLOCK: usize = DEFAULT_BLOCK_SIZE> {
    source: &'a Shrewd,
    next_start: usize,
    buffer: [i64; BLOCK],
    pos: usize,
    filled: usize
}

impl<'a, const BLOCK: usize> Iter<'a, BLOCK> {
    pub(crate) fn new(source: &'a Shrewd) -> Self {
        // With an empty buffer, decode_into would always return 0 and the iterator would
        // silently yield nothing; reject it at compile time instead.
        const { assert!(BLOCK > 0, "the iterator block size must be at least 1") };
        Self {
            source,
            next_start: 0,
            buffer: [0; BLOCK],
            pos: 0,
            filled: 0
        }
    }
}

impl<const BLOCK: usize> Iterator for Iter<'_, BLOCK> {
    type Item = i64;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos == self.filled {
            self.filled = self.source.decode_into(self.next_start, &mut self.buffer);
            self.next_start += self.filled;
            self.pos = 0;
            if self.filled == 0 {
                return None;
            }
        }
        let value = self.buffer[self.pos];
        self.pos += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.filled - self.pos) + (self.source.len() - self.next_start);
        (remaining, Some(remaining))
    }

    #[inline]
    fn fold<B, F>(mut self, init: B, mut f: F) -> B
    where
        F: FnMut(B, Self::Item) -> B,
    {
        // First, values already decoded but not yet returned (set if next() was called before).
        let mut acc = self.buffer[self.pos..self.filled].iter().fold(init, |acc, &v| f(acc, v));
        // Then the rest, one block at a time.
        loop {
            let n = self.source.decode_into(self.next_start, &mut self.buffer);
            if n == 0 {
                return acc;
            }
            self.next_start += n;
            acc = self.buffer[..n].iter().fold(acc, |acc, &v| f(acc, v));
        }
    }
}

impl<const BLOCK: usize> ExactSizeIterator for Iter<'_, BLOCK> {}
impl<const BLOCK: usize> std::iter::FusedIterator for Iter<'_, BLOCK> {}

impl<'a> IntoIterator for &'a Shrewd {
    type Item = i64;
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

