//! Errors returned when reading bytes with [`Shrewd::from_bytes`](crate::Shrewd::from_bytes).

use std::fmt;

/// The reason [`Shrewd::from_bytes`](crate::Shrewd::from_bytes) could not read its input.
///
/// New variants may be added in future releases, so a `match` on this type needs a `_` arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// A byte that should give a value width held this number. Valid widths are 1, 2, 4 and 8.
    InvalidDataSize(u8),
    /// The byte that names the encoding held this number, which is not a known encoding. This
    /// also happens when bytes from one encoding are read as another.
    InvalidCompressorType(u8),
    /// The input ended before all of the encoded data was read.
    UnexpectedEOF,
    /// The first byte held this format version, which this release cannot read. The bytes were
    /// written by a different release of this crate, or are not this crate's format at all.
    UnsupportedVersion(u8),
    /// A dictionary index held this number, which is outside the dictionary. The input is
    /// damaged.
    InvalidDictionaryIndex(u64),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDataSize(size) => write!(f, "invalid value width {size}; expected 1, 2, 4 or 8"),
            Self::InvalidCompressorType(tag) => write!(f, "unknown or unexpected encoding {tag}"),
            Self::UnexpectedEOF => f.write_str("the input ended before the encoded data did"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported format version {version}"),
            Self::InvalidDictionaryIndex(index) => write!(f, "dictionary index {index} is out of range"),
        }
    }
}

impl std::error::Error for DecodeError {}