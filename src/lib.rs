//! `shrewd-rs` stores a `Vec<i64>` in a smaller form that you can still read by index.
//!
//! Call [`Shrewd::pack`] with your values. It looks at the data once and picks one of three
//! encodings:
//!
//! * **Size** stores every value in the smallest whole-byte width that fits all of them
//!   (1, 2, 4 or 8 bytes).
//! * **Offset** stores each value as its distance from the middle of the range, which works well
//!   when the values sit in a narrow band far from zero.
//! * **Dictionary** stores each distinct value once, plus a small index for every element. It is
//!   used when there are few distinct values.
//!
//! Every element keeps a fixed position, so reading one element by index takes the same time
//! no matter where it is.
//!
//! ```
//! use shrewd_rs::Shrewd;
//!
//! let packed = Shrewd::pack(vec![1_000_000, 1_000_005, 1_000_010]);
//!
//! assert_eq!(packed.get(1), Some(1_000_005));
//! assert_eq!(packed.len(), 3);
//! assert_eq!(packed.iter().sum::<i64>(), 3_000_015);
//! ```
//!
//! The crate has no dependencies and contains no `unsafe` code.

mod shrewd;

pub use shrewd::errors::DecodeError;
pub use shrewd::iter::{Iter, DEFAULT_BLOCK_SIZE};
pub use shrewd::{CompressorType, Shrewd};
