
pub mod sizes;
pub mod compressors;
pub mod errors;
mod codec;

use sizes::{DataSize, IndexSize};
use crate::shrewd::errors::DecodeError;

pub trait Compressor: Sized {
    fn pack(v: Vec<i64>) -> Self;
    fn get(&self, index: usize) -> Option<i64>;
    fn size(&self) -> usize;
    fn length(&self) -> usize;

    fn compressor_type(&self) -> &'static str;

    fn to_bytes(&self) -> Vec<u8>;

    fn from_bytes(data: &[u8]) -> Result<Self, DecodeError>;
}

pub struct Shrewd(Inner);

pub(crate) enum Inner {
    Size(SizeCompressor),
    Offset(OffsetCompressor),
    Dictionary(DictionaryCompressor),
}

pub(crate) struct SizeCompressor {
    pub(crate) data: Vec<u8>,
    pub(crate) data_size: DataSize,
}

pub(crate) struct OffsetCompressor {
    pub(crate) data: Vec<u8>,
    pub(crate) data_size: DataSize,
    pub(crate) center: i64,
    pub(crate) original_data_size: DataSize,
}

pub(crate) struct DictionaryCompressor {
    pub(crate) data: Vec<u8>,
    pub(crate) unique_values: Vec<i64>,
    pub(crate) index_size: IndexSize,
}