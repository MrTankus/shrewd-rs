//! The [`Shrewd`] container, its encodings, and their byte format.

mod sizes;
mod compressors;
pub(crate) mod errors;
mod codec;
pub(crate) mod iter;

use std::fmt;

use sizes::{DataSize, IndexSize};
use errors::DecodeError;

/// The encoding that [`Shrewd::pack`] chose for the data.
///
/// New encodings may be added in future releases, so a `match` on this type needs a `_` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CompressorType {
    /// Each value is stored in the smallest whole-byte width that fits all values.
    Size = 1,
    /// Each value is stored as its distance from the middle of the range.
    Offset = 2,
    /// Each distinct value is stored once, with an index for every element.
    Dictionary = 3
}

impl CompressorType {
    /// Converts the encoding byte of the byte format.
    pub(crate) fn from_tag(tag: u8) -> Result<Self, DecodeError> {
        match tag {
            1 => Ok(CompressorType::Size),
            2 => Ok(CompressorType::Offset),
            3 => Ok(CompressorType::Dictionary),
            other => Err(DecodeError::InvalidCompressorType(other)),
        }
    }

    /// Returns the name of the encoding: `"Size"`, `"Offset"` or `"Dictionary"`.
    ///
    /// ```
    /// use shrewd_rs::CompressorType;
    ///
    /// assert_eq!(CompressorType::Size.as_str(), "Size");
    /// assert_eq!(CompressorType::Offset.as_str(), "Offset");
    /// assert_eq!(CompressorType::Dictionary.as_str(), "Dictionary");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            CompressorType::Size => "Size",
            CompressorType::Offset => "Offset",
            CompressorType::Dictionary => "Dictionary",
        }
    }
}

impl fmt::Display for CompressorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// The operations every encoding provides. It is private to the crate: users call the methods on
// `Shrewd`, which forward to the encoding it holds.
pub(crate) trait Compressor: Sized {
    fn pack(v: Vec<i64>) -> Self;
    fn get(&self, index: usize) -> Option<i64>;
    fn memory_size(&self) -> usize;
    fn len(&self) -> usize;
    fn compressor_type(&self) -> CompressorType;
    fn to_bytes(&self) -> Vec<u8>;
    fn from_bytes(data: &[u8]) -> Result<Self, DecodeError>;
    fn decode_into(&self, start: usize, out: &mut [i64]) -> usize;
}

/// A compressed, read-only vector of `i64` values.
///
/// Build one with [`Shrewd::pack`], which picks the encoding that uses the least memory for
/// your data. Once built, it cannot be changed. To change the values, decode them, change them,
/// and pack them again.
///
/// ```
/// use shrewd_rs::{CompressorType, Shrewd};
///
/// // Two distinct values repeated many times, so each is stored once.
/// let values: Vec<i64> = [-5_000_000_000, 7_000_000_000].repeat(4);
/// let packed = Shrewd::pack(values);
/// assert_eq!(packed.compressor_type(), CompressorType::Dictionary);
///
/// for value in &packed {
///     assert!(value == -5_000_000_000 || value == 7_000_000_000);
/// }
/// ```
#[derive(Debug)]
pub struct Shrewd(Inner);

impl Shrewd {
    /// Compresses `v`, taking ownership of it.
    ///
    /// The input vector is freed while the result is built, so the full data is never held
    /// twice. An empty vector is valid and gives a result with a length of 0.
    ///
    /// ```
    /// use shrewd_rs::Shrewd;
    ///
    /// let packed = Shrewd::pack(vec![10, 20, 30]);
    /// assert_eq!(packed.len(), 3);
    /// ```
    pub fn pack(v: Vec<i64>) -> Self {
        Compressor::pack(v)
    }

    /// Returns the value at `index`, or `None` if `index` is not less than [`len`](Self::len).
    ///
    /// ```
    /// use shrewd_rs::Shrewd;
    ///
    /// let packed = Shrewd::pack(vec![10, 20, 30]);
    /// assert_eq!(packed.get(2), Some(30));
    /// assert_eq!(packed.get(3), None);
    /// ```
    #[inline]
    pub fn get(&self, index: usize) -> Option<i64> {
        Compressor::get(self, index)
    }

    /// Returns the memory used, in bytes: the heap memory that holds the data, plus the size of
    /// the value itself.
    ///
    /// ```
    /// use shrewd_rs::Shrewd;
    ///
    /// // 1,000 values that each fit in 1 byte, compared with 8,000 bytes in a Vec<i64>.
    /// let packed = Shrewd::pack((0..1_000).map(|x| x % 100).collect());
    /// assert!(packed.memory_size() < 1_100);
    /// ```
    pub fn memory_size(&self) -> usize {
        Compressor::memory_size(self)
    }

    /// Returns the number of values.
    ///
    /// ```
    /// use shrewd_rs::Shrewd;
    ///
    /// assert_eq!(Shrewd::pack(vec![4, 5, 6]).len(), 3);
    /// ```
    #[inline]
    pub fn len(&self) -> usize {
        Compressor::len(self)
    }

    /// Returns `true` if there are no values.
    ///
    /// ```
    /// use shrewd_rs::Shrewd;
    ///
    /// assert!(Shrewd::pack(Vec::new()).is_empty());
    /// assert!(!Shrewd::pack(vec![1]).is_empty());
    /// ```
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the encoding that [`pack`](Self::pack) chose.
    ///
    /// The encoding cannot be set by the caller. This method is useful for logging and for
    /// understanding performance.
    ///
    /// ```
    /// use shrewd_rs::{CompressorType, Shrewd};
    ///
    /// // Values that are small enough to store in fewer bytes.
    /// assert_eq!(Shrewd::pack(vec![-3, 120, 7, 64]).compressor_type(), CompressorType::Size);
    ///
    /// // Values in a narrow band far from zero.
    /// let band: Vec<i64> = (0..100).map(|x| 9_000_000_000 + x).collect();
    /// assert_eq!(Shrewd::pack(band).compressor_type(), CompressorType::Offset);
    /// ```
    pub fn compressor_type(&self) -> CompressorType {
        Compressor::compressor_type(self)
    }

    /// Encodes the values as bytes that [`from_bytes`](Self::from_bytes) can read back.
    ///
    /// The layout is the same on every platform. All numbers are little-endian, and all lengths
    /// are 8-byte `u64` values, so bytes written on a 64-bit system can be read on a 32-bit one.
    ///
    /// Every encoding starts with two bytes: the format version (currently 1) and the encoding
    /// (1 = Size, 2 = Offset, 3 = Dictionary). A release that changes the layout will also change
    /// the version, so older or newer bytes are rejected instead of being read wrongly. The rest
    /// depends on the encoding:
    ///
    /// | Encoding | After the two header bytes |
    /// | :--- | :--- |
    /// | Size | value width (1 byte), number of values (`u64`), the values |
    /// | Offset | value width (1 byte), number of values (`u64`), center (`i64`), each value minus the center |
    /// | Dictionary | index width (1 byte), table length (`u64`), the table (`i64` each), number of values (`u64`), one index per value |
    ///
    /// Value and index widths are 1, 2, 4 or 8 bytes.
    ///
    /// ```
    /// use shrewd_rs::Shrewd;
    ///
    /// let bytes = Shrewd::pack(vec![1, 2, 3]).to_bytes();
    ///
    /// // Format version 1, then encoding 1 (Size).
    /// assert_eq!(&bytes[..2], &[1, 1]);
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        Compressor::to_bytes(self)
    }

    /// Reads values from bytes written by [`to_bytes`](Self::to_bytes).
    ///
    /// Lengths in the input are checked against the real input size before any memory is
    /// allocated, so a damaged length cannot cause a huge allocation.
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] if the input is too short, was written in a format version this
    /// release cannot read, names an unknown encoding or width, or contains a dictionary index
    /// outside the dictionary. See [`DecodeError`] for the cases.
    ///
    /// Every value that this method accepts can be read without a panic. Bytes after the end of
    /// the encoded data are ignored.
    ///
    /// ```
    /// use shrewd_rs::{DecodeError, Shrewd};
    ///
    /// let bytes = Shrewd::pack(vec![10, 20, 30]).to_bytes();
    ///
    /// let restored = Shrewd::from_bytes(&bytes)?;
    /// assert_eq!(restored.get(1), Some(20));
    ///
    /// // Input that ends too early is reported as an error.
    /// assert_eq!(Shrewd::from_bytes(&bytes[..4]).err(), Some(DecodeError::UnexpectedEOF));
    /// # Ok::<(), DecodeError>(())
    /// ```
    pub fn from_bytes(data: &[u8]) -> Result<Self, DecodeError> {
        Compressor::from_bytes(data)
    }

    /// Decodes values starting at `start` into `out`, and returns how many values were written.
    ///
    /// It writes `out.len()` values, or fewer if the data ends first. If `start` is not less than
    /// [`len`](Self::len), it writes nothing and returns 0. Values in `out` after the returned
    /// count are left unchanged. This method does not allocate.
    ///
    /// This is the fastest way to read many values in a row.
    ///
    /// ```
    /// use shrewd_rs::Shrewd;
    ///
    /// let packed = Shrewd::pack((0..100).collect());
    /// let mut buffer = [0i64; 16];
    ///
    /// assert_eq!(packed.decode_into(10, &mut buffer), 16);
    /// assert_eq!(buffer[0], 10);
    ///
    /// // Only 4 values are left after index 96.
    /// assert_eq!(packed.decode_into(96, &mut buffer), 4);
    /// ```
    #[inline]
    pub fn decode_into(&self, start: usize, out: &mut [i64]) -> usize {
        Compressor::decode_into(self, start, out)
    }

    /// Returns an iterator over the values, in order.
    ///
    /// The iterator decodes [`DEFAULT_BLOCK_SIZE`](crate::DEFAULT_BLOCK_SIZE) values at a time
    /// into a small buffer. A plain `for` loop works, but methods such as `fold`, `sum` and
    /// `for_each` are faster, because they decode and process whole blocks at once.
    ///
    /// ```
    /// use shrewd_rs::Shrewd;
    ///
    /// let packed = Shrewd::pack(vec![1, 2, 3]);
    /// assert_eq!(packed.iter().sum::<i64>(), 6);
    /// ```
    pub fn iter(&self) -> iter::Iter<'_> {
        iter::Iter::new(self)
    }

    /// Returns an iterator like [`iter`](Self::iter) that decodes `BLOCK` values at a time.
    ///
    /// A larger block, such as 1024, can make `fold`, `sum` and `for_each` much faster on
    /// desktop and server CPUs, especially when the program is built for the target CPU
    /// (`-C target-cpu=native`). It also makes starting an iterator slower, because the first
    /// block is decoded before the first value is returned. The iterator holds the block on the
    /// stack, so it takes `BLOCK * 8` bytes. A block size of 0 does not compile.
    ///
    /// ```
    /// use shrewd_rs::Shrewd;
    ///
    /// let packed = Shrewd::pack((0..10_000).collect());
    /// let total = packed.iter_buffered::<1024>().fold(0i64, |sum, value| sum + value);
    /// assert_eq!(total, 49_995_000);
    /// ```
    pub fn iter_buffered<const BLOCK: usize>(&self) -> iter::Iter<'_, BLOCK> {
        iter::Iter::new(self)
    }
}

/// Compresses a vector, taking ownership of it. This is the same as [`Shrewd::pack`].
///
/// ```
/// use shrewd_rs::Shrewd;
///
/// let values: Vec<i64> = vec![10, 20, 30];
/// let packed: Shrewd = values.into();
/// assert_eq!(packed.get(1), Some(20));
///
/// // `values` has been moved into `packed` and can no longer be used.
/// // Clone it first if you still need it: `Shrewd::from(values.clone())`.
/// ```
impl From<Vec<i64>> for Shrewd {
    fn from(values: Vec<i64>) -> Self {
        Shrewd::pack(values)
    }
}

/// Collects an iterator into a `Shrewd`.
///
/// `pack` needs all the values before it can choose an encoding, so the values are first
/// collected into a `Vec<i64>` and then packed. This costs the same as building the vector
/// yourself and calling [`Shrewd::pack`].
///
/// Collecting from an iterator that borrows a vector, such as `values.iter().copied()`, keeps
/// the original vector alive, so memory peaks at two full copies plus the packed result. Use
/// `values.into_iter()` instead when you no longer need the vector: its buffer is reused, so no
/// extra copy is made.
///
/// ```
/// use shrewd_rs::Shrewd;
///
/// // From any iterator of i64 values.
/// let packed: Shrewd = (0..1_000).map(|x| x * 3).collect();
/// assert_eq!(packed.get(10), Some(30));
///
/// // From a vector you no longer need, without an extra copy.
/// let values: Vec<i64> = vec![5, 6, 7];
/// let packed: Shrewd = values.into_iter().filter(|&x| x != 6).collect();
/// assert_eq!(packed.len(), 2);
/// ```
impl FromIterator<i64> for Shrewd {
    fn from_iter<I: IntoIterator<Item = i64>>(iter: I) -> Self {
        Shrewd::pack(iter.into_iter().collect())
    }
}

#[derive(Debug)]
pub(crate) enum Inner {
    Size(SizeCompressor),
    Offset(OffsetCompressor),
    Dictionary(DictionaryCompressor),
}

#[derive(Debug)]
pub(crate) struct SizeCompressor {
    pub(crate) data: Vec<u8>,
    pub(crate) data_size: DataSize,
}

#[derive(Debug)]
pub(crate) struct OffsetCompressor {
    pub(crate) data: Vec<u8>,
    pub(crate) data_size: DataSize,
    pub(crate) center: i64,
}

#[derive(Debug)]
pub(crate) struct DictionaryCompressor {
    pub(crate) data: Vec<u8>,
    pub(crate) unique_values: Vec<i64>,
    pub(crate) index_size: IndexSize,
}