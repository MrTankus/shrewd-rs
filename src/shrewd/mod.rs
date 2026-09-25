
pub mod sizes;
pub mod compressors;

use sizes::{DataSize, IndexSize};

pub trait Compressor {
    fn pack(v: Vec<i64>) -> Self;
    fn get(&self, index: usize) -> i64;
    fn size(&self) -> usize;
    fn length(&self) -> usize;

    fn compressor_type(&self) -> String;
}

pub enum Shrewd {
    Size(SizeCompressor),
    Average(AverageCompressor),
    Dictionary(DictionaryCompressor),
}

pub struct SizeCompressor {
    pub(crate) data: Vec<u8>,
    pub(crate) data_size: DataSize,
}

pub struct AverageCompressor {
    pub(crate) data: Vec<u8>,
    pub(crate) data_size: DataSize,
    pub(crate) avg: i64,
    pub(crate) original_data_size: DataSize,
}

pub struct DictionaryCompressor {
    pub(crate) data: Vec<u8>,
    pub(crate) unique_values: Vec<i64>,
    pub(crate) index_size: IndexSize,
}