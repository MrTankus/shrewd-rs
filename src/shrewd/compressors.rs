use std::cmp::max;
use std::mem::{size_of, size_of_val};
use std::collections::{HashMap};
use super::sizes::{HyperLogLog, DataSize, IndexSize};
use super::{Compressor, Shrewd, Inner, SizeCompressor, OffsetCompressor, DictionaryCompressor, CompressorType};
use super::errors::DecodeError;
use super::codec::Reader;

/// Written as the first byte of every encoding. Change it whenever the byte layout changes, so
/// that bytes in an older or newer layout are rejected instead of misread.
pub(crate) const FORMAT_VERSION: u8 = 1;

const HEADER_SIZE: usize = 2;
const LEN_SIZE: usize = size_of::<u64>();

fn write_header(out: &mut Vec<u8>, compressor_type: CompressorType) {
    out.push(FORMAT_VERSION);
    out.push(compressor_type as u8);
}

/// Lengths are always written as 8 bytes, so the layout is the same on 32-bit and 64-bit systems.
fn write_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&(len as u64).to_le_bytes());
}

fn read_header(reader: &mut Reader) -> Result<CompressorType, DecodeError> {
    match reader.read_u8()? {
        FORMAT_VERSION => CompressorType::from_tag(reader.read_u8()?),
        other => Err(DecodeError::UnsupportedVersion(other)),
    }
}

fn expect_header(reader: &mut Reader, expected: CompressorType) -> Result<(), DecodeError> {
    match read_header(reader)? as u8 {
        tag if tag == expected as u8 => Ok(()),
        other => Err(DecodeError::InvalidCompressorType(other)),
    }
}

impl Compressor for Shrewd {
    fn pack(v: Vec<i64>) -> Self {
        if v.is_empty() {
            return Shrewd(Inner::Size(SizeCompressor::pack(v)));
        }

        // --- PHASE 1: LIGHTWEIGHT ESTIMATION PASS (O(N)) ---
        let mut min_val = v[0];
        let mut max_val = v[0];

        let mut hll = HyperLogLog::new(12);

        for &value in v.iter() {
            if value < min_val {
                min_val = value;
            }
            if value > max_val {
                max_val = value;
            }

            hll.insert(value);
        }

        let original_data_size = max(DataSize::calculate(min_val), DataSize::calculate(max_val));
        let (center, max_delta_from_center) = OffsetCompressor::midpoint_and_width(min_val, max_val);

        // --- PHASE 2: CALCULATE PRECISE METRIC FOOTPRINTS (O(1)) ---
        // 1. SizeCompressor Footprint
        let size_est_bytes = v.len() * original_data_size as u8 as usize;

        // 2. OffsetCompressor Footprint
        let offset_est_bytes = v.len() * max_delta_from_center as u8 as usize;

        // 3. DictionaryCompressor Footprint
        let unique_count = hll.estimate().min(v.len()); // Highly scalable O(1) extraction
        let dict_idx_width = IndexSize::calculate(if unique_count > 0 {
            unique_count - 1
        } else {
            0
        });
        let dict_est_bytes =
            (v.len() * dict_idx_width as u8 as usize) + (unique_count * size_of::<i64>());

        // --- PHASE 3: EVALUATE WINNER AND DISPATCH INITIALIZATION ---
        let low_uniqueness = unique_count < v.len() / 2;

        if low_uniqueness && dict_est_bytes < size_est_bytes && dict_est_bytes < offset_est_bytes {
            Shrewd(Inner::Dictionary(DictionaryCompressor::_compress(v, unique_count)))
        } else if offset_est_bytes < size_est_bytes {
            Shrewd(Inner::Offset(OffsetCompressor::_compress(v, center, max_delta_from_center)))
        } else {
            Shrewd(Inner::Size(SizeCompressor::_compress(v, original_data_size)))
        }
    }

    #[inline(always)]
    fn get(&self, index: usize) -> Option<i64> {
        match &self.0 {
            Inner::Size(c) => c.get(index),
            Inner::Offset(c) => c.get(index),
            Inner::Dictionary(c) => c.get(index),
        }
    }

    #[inline]
    fn memory_size(&self) -> usize {
        // Includes the small stack-overhead variant size of the tracking enum container itself
        match &self.0 {
            Inner::Size(c) => c.memory_size() - size_of_val(c) + size_of_val(self),
            Inner::Offset(c) => c.memory_size() - size_of_val(c) + size_of_val(self),
            Inner::Dictionary(c) => c.memory_size() - size_of_val(c) + size_of_val(self),
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match &self.0 {
            Inner::Size(c) => c.len(),
            Inner::Offset(c) => c.len(),
            Inner::Dictionary(c) => c.len(),
        }
    }

    #[inline(always)]
    fn compressor_type(&self) -> CompressorType {
        match &self.0 {
            Inner::Size(c) => c.compressor_type(),
            Inner::Offset(c) => c.compressor_type(),
            Inner::Dictionary(c) => c.compressor_type(),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        match &self.0 {
            Inner::Size(c) => c.to_bytes(),
            Inner::Offset(c) => c.to_bytes(),
            Inner::Dictionary(c) => c.to_bytes(),
        }
    }

    fn from_bytes(data: &[u8]) -> Result<Self, DecodeError> {
        match read_header(&mut Reader::new(data))? {
            CompressorType::Size => SizeCompressor::from_bytes(data).map(|c| Shrewd(Inner::Size(c))),
            CompressorType::Offset => OffsetCompressor::from_bytes(data).map(|c| Shrewd(Inner::Offset(c))),
            CompressorType::Dictionary => DictionaryCompressor::from_bytes(data).map(|c| Shrewd(Inner::Dictionary(c))),
        }
    }

    fn decode_into(&self, start: usize, out: &mut [i64]) -> usize {
        match &self.0 {
            Inner::Size(c) => c.decode_into(start, out),
            Inner::Offset(c) => c.decode_into(start, out),
            Inner::Dictionary(c) => c.decode_into(start, out),
        }
    }
}


impl SizeCompressor {
    pub(crate) fn _compress(v: Vec<i64>, data_size:DataSize) -> Self {
        let mut data: Vec<u8> = Vec::with_capacity(v.len() * data_size as usize);

        match data_size {
            DataSize::I8 => {
                for value in v {
                    data.push(value as i8 as u8);
                }
            }
            DataSize::I16 => {
                for value in v {
                    data.extend_from_slice(&(value as i16).to_le_bytes());
                }
            }
            DataSize::I32 => {
                for value in v {
                    data.extend_from_slice(&(value as i32).to_le_bytes());
                }
            }
            DataSize::I64 => {
                for value in v {
                    data.extend_from_slice(&value.to_le_bytes());
                }
            }
        }

        SizeCompressor {
            data,
            data_size,
        }
    }
}

impl Compressor for SizeCompressor {

    fn pack(v: Vec<i64>) -> Self {
        let mut size = DataSize::I8;
        for &value in v.iter() {
            let value_data_size = DataSize::calculate(value);
            if value_data_size > size {
                size = value_data_size;
                if size == DataSize::I64 {
                    break; // No need to check the rest; it's already forced to max size
                }
            }
        }

        Self::_compress(v, size)
    }

    fn get(&self, index: usize) -> Option<i64> {
        if index >= self.len() {
            return None;
        }
        let value = match self.data_size {
            DataSize::I8 => self.data[index] as i8 as i64,
            DataSize::I16 => {
                let real_index = index * DataSize::I16 as usize;
                let val = i16::from_le_bytes(
                    self.data[real_index..(real_index + DataSize::I16 as usize)]
                        .try_into()
                        .unwrap(),
                );
                val as i64
            }
            DataSize::I32 => {
                let real_index = index * DataSize::I32 as usize;
                let val = i32::from_le_bytes(
                    self.data[real_index..(real_index + DataSize::I32 as usize)]
                        .try_into()
                        .unwrap(),
                );
                val as i64
            }
            DataSize::I64 => {
                let real_index = index * DataSize::I64 as usize;
                i64::from_le_bytes(
                    self.data[real_index..(real_index + DataSize::I64 as usize)]
                        .try_into()
                        .unwrap(),
                )
            }
        };
        Some(value)
    }

    fn memory_size(&self) -> usize {
        self.data.len() + size_of_val(self)
    }

    #[inline]
    fn len(&self) -> usize {
        self.data.len() >> self.data_size.shift_factor()
    }

    #[inline(always)]
    fn compressor_type(&self) -> CompressorType {
        CompressorType::Size
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_SIZE + size_of::<u8>() + LEN_SIZE + self.data.len());
        write_header(&mut bytes, CompressorType::Size);
        bytes.push(self.data_size as u8);
        write_len(&mut bytes, self.len());
        bytes.extend_from_slice(&self.data);
        bytes
    }

    fn from_bytes(data: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = Reader::new(data);
        expect_header(&mut reader, CompressorType::Size)?;
        let data_size: DataSize = reader.read_u8()?.try_into()?;
        let length = reader.read_len()?;
        let data = reader.read_packed(length, data_size as usize)?.to_vec();
        Ok(Self {
            data_size,
            data,
        })
    }

    fn decode_into(&self, start: usize, out: &mut [i64]) -> usize {
        let n = out.len().min(self.len().saturating_sub(start));
        if n == 0 {
            return 0;
        }
        let w = self.data_size as usize;
        let bytes = &self.data[start * w..(start + n) * w];
        match self.data_size {
            DataSize::I8 => {
                for (index, val) in bytes.iter().enumerate() {
                    out[index] = *val as i8 as i64;
                }
                bytes.len()
            },
            DataSize::I16 => {
                let (src, _) = bytes.as_chunks::<2>();
                for (index, val) in src.iter().enumerate() {
                    out[index] = i16::from_le_bytes(*val) as i64;
                }
                src.len()
            },
            DataSize::I32 => {
                let (src, _) = bytes.as_chunks::<4>();
                for (index, val) in src.iter().enumerate() {
                    out[index] = i32::from_le_bytes(*val) as i64;
                }
                src.len()
            },
            DataSize::I64 => {
                let (src, _) = bytes.as_chunks::<8>();
                for (index, val) in src.iter().enumerate() {
                    out[index] = i64::from_le_bytes(*val);
                }
                src.len()
            }
        }
    }
}



impl OffsetCompressor {
    pub(crate) fn midpoint_and_width(min_val: i64, max_val: i64) -> (i64, DataSize) {
        let mid = ((min_val as i128 + max_val as i128 + 1) >> 1) as i64;
        let range = (max_val as i128 - min_val as i128) as u64;
        let width = if range <= u8::MAX as u64 {
            DataSize::I8
        } else if range <= u16::MAX as u64 {
            DataSize::I16
        } else if range <= u32::MAX as u64 {
            DataSize::I32
        } else {
            DataSize::I64
        };
        (mid, width)
    }

    pub(crate) fn _compress(v: Vec<i64>, center: i64, max_delta_from_center: DataSize) -> Self {
        let cap = match max_delta_from_center {
            DataSize::I8 => v.len(),
            DataSize::I16 => size_of::<i16>() * v.len(),
            DataSize::I32 => size_of::<i32>() * v.len(),
            DataSize::I64 => size_of::<i64>() * v.len(),
        };

        let mut compressed_data: Vec<u8> = Vec::with_capacity(cap);

        for value in v.into_iter() {
            // Truncating to the delta width below keeps the same low bits whether the subtraction
            // wraps at 64 bits or at the original width, so plain i64 arithmetic is enough.
            let transformed_value = value.wrapping_sub(center);
            match max_delta_from_center {
                DataSize::I8 => {
                    compressed_data.extend_from_slice(&(transformed_value as i8).to_le_bytes());
                }
                DataSize::I16 => {
                    compressed_data.extend_from_slice(&(transformed_value as i16).to_le_bytes());
                }
                DataSize::I32 => {
                    compressed_data.extend_from_slice(&(transformed_value as i32).to_le_bytes());
                }
                DataSize::I64 => {
                    compressed_data.extend_from_slice(&transformed_value.to_le_bytes());
                }
            }
        }

        OffsetCompressor {
            data: compressed_data,
            data_size: max_delta_from_center,
            center,
        }
    }
}

impl Compressor for OffsetCompressor {
    fn pack(v: Vec<i64>) -> Self {
        if v.is_empty() {
            return OffsetCompressor {
                data: vec![],
                data_size: DataSize::I8,
                center: 0,
            };
        }
        let mut min_val = i64::MAX;
        let mut max_val = i64::MIN;
        for &value in v.iter() {
            min_val = min_val.min(value);
            max_val = max_val.max(value);
        }
        let (center, size) = Self::midpoint_and_width(min_val, max_val);
        Self::_compress(v, center, size)
    }

    fn get(&self, index: usize) -> Option<i64> {
        if index >= self.len() {
            return None;
        }
        let transformed_value = match self.data_size {
            DataSize::I8 => self.data[index] as i8 as i64,
            DataSize::I16 => {
                let real_index = index * DataSize::I16 as usize;
                i16::from_le_bytes(
                    self.data[real_index..(real_index + DataSize::I16 as usize)]
                        .try_into()
                        .unwrap(),
                ) as i64
            }
            DataSize::I32 => {
                let real_index = index * DataSize::I32 as usize;
                i32::from_le_bytes(
                    self.data[real_index..(real_index + DataSize::I32 as usize)]
                        .try_into()
                        .unwrap(),
                ) as i64
            }
            DataSize::I64 => {
                let real_index = index * DataSize::I64 as usize;
                i64::from_le_bytes(
                    self.data[real_index..(real_index + DataSize::I64 as usize)]
                        .try_into()
                        .unwrap(),
                )
            }
        };

        // The center is the midpoint of min and max, so delta + center is exactly the original
        // value; no wrapping at the original width is needed.
        Some(transformed_value.wrapping_add(self.center))
    }

    fn memory_size(&self) -> usize {
        // size of vec data + vec pointer + size enum + center
        self.data.len() + size_of_val(self)
    }

    #[inline]
    fn len(&self) -> usize {
        self.data.len() >> self.data_size.shift_factor()
    }

    #[inline(always)]
    fn compressor_type(&self) -> CompressorType {
        CompressorType::Offset
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_SIZE + size_of::<u8>() + LEN_SIZE + DataSize::I64 as usize + self.data.len());
        write_header(&mut out, CompressorType::Offset);
        out.push(self.data_size as u8);
        write_len(&mut out, self.len());
        out.extend_from_slice(&self.center.to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    fn from_bytes(data: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = Reader::new(data);
        expect_header(&mut reader, CompressorType::Offset)?;
        let data_size: DataSize = reader.read_u8()?.try_into()?;
        let length = reader.read_len()?;
        let center = reader.read_i64()?;
        let data = reader.read_packed(length, data_size as usize)?.to_vec();
        Ok(Self {
            data_size,
            center,
            data,
        })
    }

    fn decode_into(&self, start: usize, out: &mut [i64]) -> usize {
        let n = out.len().min(self.len().saturating_sub(start));
        if n == 0 {
            return 0;
        }
        let w = self.data_size as usize;
        let bytes = &self.data[start * w..(start + n) * w];
        match self.data_size {
            DataSize::I8 => {
                for (index, val) in bytes.iter().enumerate() {
                    out[index] = (*val as i8 as i64).wrapping_add(self.center);
                }
            },
            DataSize::I16 => {
                let (src, _) = bytes.as_chunks::<2>();
                for (index, val) in src.iter().enumerate() {
                    out[index] = (i16::from_le_bytes(*val) as i64).wrapping_add(self.center);
                }
            },
            DataSize::I32 => {
                let (src, _) = bytes.as_chunks::<4>();
                for (index, val) in src.iter().enumerate() {
                    out[index] = (i32::from_le_bytes(*val) as i64).wrapping_add(self.center);
                }
            },
            DataSize::I64 => {
                let (src, _) = bytes.as_chunks::<8>();
                for (index, val) in src.iter().enumerate() {
                    out[index] = i64::from_le_bytes(*val).wrapping_add(self.center);
                }
            }
        }
        n
    }
}


impl DictionaryCompressor {
    /// The largest index in `indices`, or `None` if there are no indices.
    fn largest_index(indices: &[u8], index_size: IndexSize) -> Option<u64> {
        match index_size {
            IndexSize::U8 => indices.iter().copied().max().map(u64::from),
            IndexSize::U16 => indices.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).max().map(u64::from),
            IndexSize::U32 => indices.as_chunks::<4>().0.iter().map(|&c| u32::from_le_bytes(c)).max().map(u64::from),
            IndexSize::U64 => indices.as_chunks::<8>().0.iter().map(|&c| u64::from_le_bytes(c)).max(),
        }
    }

    pub(crate) fn _compress(v: Vec<i64>, unique_count_estimation: usize) -> Self {
        if v.is_empty() {
            return DictionaryCompressor {
                data: vec![],
                unique_values: vec![],
                index_size: IndexSize::U8,
            };
        }
        let mut lookup: HashMap<i64, usize> = HashMap::with_capacity(unique_count_estimation);
        let mut index = 0usize;
        for &value in v.iter() {
            lookup.entry(value).or_insert_with(|| {
                let idx = index;
                index += 1;
                idx
            });
        }

        let mut unique_values = vec![0i64; lookup.len()];
        for (&k, &v) in &lookup {
            unique_values[v] = k;
        }

        let data_size = IndexSize::calculate(unique_values.len() - 1);
        let mut data: Vec<u8> = Vec::with_capacity(v.len() * data_size as usize);
        match data_size {
            IndexSize::U8 => {
                for value in v.into_iter() {
                    let index = lookup[&value];
                    data.push(index as u8);
                }
            }
            IndexSize::U16 => {
                for value in v.into_iter() {
                    let index = lookup[&value];
                    data.extend_from_slice(&((index as u16).to_le_bytes()));
                }
            }
            IndexSize::U32 => {
                for value in v.into_iter() {
                    let index = lookup[&value];
                    data.extend_from_slice(&((index as u32).to_le_bytes()));
                }
            }
            IndexSize::U64 => {
                for value in v.into_iter() {
                    let index = lookup[&value];
                    data.extend_from_slice(&((index as u64).to_le_bytes()));
                }
            }
        }

        DictionaryCompressor {
            data,
            unique_values,
            index_size: data_size,
        }
    }
}
impl Compressor for DictionaryCompressor {
    fn pack(v: Vec<i64>) -> Self {
        Self::_compress(v, 0)
    }

    fn get(&self, index: usize) -> Option<i64> {
        if index >= self.len() {
            return None;
        }
        let value = match self.index_size {
            IndexSize::U8 => {
                let dict_index = self.data[index];
                self.unique_values[dict_index as usize]
            }
            IndexSize::U16 => {
                let real_index = index * IndexSize::U16 as usize;
                let dict_index: u16 = u16::from_le_bytes(
                    self.data[real_index..(real_index + IndexSize::U16 as usize)]
                        .try_into()
                        .unwrap(),
                );
                self.unique_values[dict_index as usize]
            }
            IndexSize::U32 => {
                let real_index = index * IndexSize::U32 as usize;
                let dict_index: u32 = u32::from_le_bytes(
                    self.data[real_index..(real_index + IndexSize::U32 as usize)]
                        .try_into()
                        .unwrap(),
                );
                self.unique_values[dict_index as usize]
            }
            IndexSize::U64 => {
                let real_index = index * IndexSize::U64 as usize;
                let dict_index: u64 = u64::from_le_bytes(
                    self.data[real_index..(real_index + IndexSize::U64 as usize)]
                        .try_into()
                        .unwrap(),
                );
                self.unique_values[dict_index as usize]
            }
        };
        Some(value)
    }

    #[inline]
    fn len(&self) -> usize {
        self.data.len() >> self.index_size.shift_factor()
    }

    fn decode_into(&self, start: usize, out: &mut [i64]) -> usize {
        let n = out.len().min(self.len().saturating_sub(start));
        if n == 0 {
            return 0;
        }
        let w = self.index_size as usize;
        let bytes = &self.data[start * w..(start + n) * w];
        let unique_values = &self.unique_values;
        match self.index_size {
            IndexSize::U8 => {
                for (index, val) in bytes.iter().enumerate() {
                    out[index] = unique_values[*val as usize];
                }
            },
            IndexSize::U16 => {
                let (src, _) = bytes.as_chunks::<2>();
                for (index, val) in src.iter().enumerate() {
                    out[index] = unique_values[u16::from_le_bytes(*val) as usize];
                }
            },
            IndexSize::U32 => {
                let (src, _) = bytes.as_chunks::<4>();
                for (index, val) in src.iter().enumerate() {
                    out[index] = unique_values[u32::from_le_bytes(*val) as usize];
                }
            },
            IndexSize::U64 => {
                let (src, _) = bytes.as_chunks::<8>();
                for (index, val) in src.iter().enumerate() {
                    out[index] = unique_values[u64::from_le_bytes(*val) as usize];
                }
            }
        }
        n
    }

    fn memory_size(&self) -> usize {
        self.data.len() + (self.unique_values.len() * DataSize::I64 as usize) + size_of_val(self)
    }

    #[inline(always)]
    fn compressor_type(&self) -> CompressorType {
        CompressorType::Dictionary
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_SIZE + size_of::<u8>() + LEN_SIZE + (self.unique_values.len() * DataSize::I64 as usize) + LEN_SIZE + self.data.len());
        write_header(&mut bytes, CompressorType::Dictionary);
        bytes.push(self.index_size as u8);
        write_len(&mut bytes, self.unique_values.len());
        for &unique_value in &self.unique_values {
            bytes.extend_from_slice(&unique_value.to_le_bytes());
        }
        write_len(&mut bytes, self.len());
        bytes.extend_from_slice(&self.data);
        bytes
    }

    fn from_bytes(data: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = Reader::new(data);
        expect_header(&mut reader, CompressorType::Dictionary)?;
        let index_size: IndexSize = reader.read_u8()?.try_into()?;
        let unique_values_length = reader.read_len()?;
        let unique_values: Vec<i64> = reader
            .read_packed(unique_values_length, DataSize::I64 as usize)?
            .as_chunks::<{ DataSize::I64 as usize }>()
            .0
            .iter()
            .map(|&chunk| i64::from_le_bytes(chunk))
            .collect();
        let element_count = reader.read_len()?;
        let indices = reader.read_packed(element_count, index_size as usize)?;
        // `get` and `decode_into` look indices up without checking them, so every index must
        // point into the table. Checking the largest one is enough.
        if let Some(largest) = Self::largest_index(indices, index_size)
            && largest >= unique_values.len() as u64
        {
            return Err(DecodeError::InvalidDictionaryIndex(largest));
        }
        let data = indices.to_vec();
        Ok(Self {
            index_size,
            unique_values,
            data,
        })
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::shrewd::iter::DEFAULT_BLOCK_SIZE as BLOCK_SIZE;
    use rand::distr::{Distribution, StandardUniform};

    fn generate_random_vecs(size: usize) -> (Vec<i64>, Vec<i64>, Vec<i64>, Vec<i64>) {
        let mut rng = rand::rng();
        let random_i8s: Vec<i8> = StandardUniform.sample_iter(&mut rng).take(size).collect();
        let random_i8s: Vec<i64> = random_i8s.into_iter().map(i64::from).collect();

        let random_i16s: Vec<i16> = StandardUniform.sample_iter(&mut rng).take(size).collect();
        let random_i16s: Vec<i64> = random_i16s.into_iter().map(i64::from).collect();

        let random_i32s: Vec<i32> = StandardUniform.sample_iter(&mut rng).take(size).collect();
        let random_i32s: Vec<i64> = random_i32s.into_iter().map(i64::from).collect();

        let random_i64s: Vec<i64> = StandardUniform.sample_iter(&mut rng).take(size).collect();

        (random_i8s, random_i16s, random_i32s, random_i64s)
    }

    #[test]
    fn test_size_compressor_basic_fields() {
        let (random_i8s, random_i16s, random_i32s, random_i64s) = generate_random_vecs(1000);

        let compressor = SizeCompressor::pack(random_i8s.clone());
        assert_eq!(compressor.data_size, DataSize::I8);
        assert_eq!(compressor.len(), random_i8s.len());
        assert_eq!(compressor.memory_size(), size_of_val(&compressor) + (random_i8s.len() * DataSize::I8 as usize));

        let compressor = SizeCompressor::pack(random_i16s.clone());
        assert_eq!(compressor.data_size, DataSize::I16);
        assert_eq!(compressor.len(), random_i16s.len());
        assert_eq!(compressor.memory_size(), size_of_val(&compressor) + (random_i16s.len() * DataSize::I16 as usize));

        let compressor = SizeCompressor::pack(random_i32s.clone());
        assert_eq!(compressor.data_size, DataSize::I32);
        assert_eq!(compressor.len(), random_i32s.len());
        assert_eq!(compressor.memory_size(), size_of_val(&compressor) + (random_i32s.len() * DataSize::I32 as usize));

        let compressor = SizeCompressor::pack(random_i64s.clone());
        assert_eq!(compressor.data_size, DataSize::I64);
        assert_eq!(compressor.len(), random_i64s.len());
        assert_eq!(compressor.memory_size(), size_of_val(&compressor) + (random_i64s.len() * DataSize::I64 as usize));
    }

    #[test]
    fn test_offset_compressor_basic_fields() {
        let (random_i8s, random_i16s, random_i32s, random_i64s) = generate_random_vecs(10);

        let compressor = OffsetCompressor::pack(random_i8s.clone());
        println!("i8 vec: {:?}", random_i8s);
        assert_eq!(compressor.data_size, DataSize::I8);
        assert_eq!(compressor.len(), random_i8s.len());

        let compressor = OffsetCompressor::pack(random_i16s.clone());
        println!("i16 vec: {:?}", random_i16s);
        assert_eq!(compressor.data_size, DataSize::I16);
        assert_eq!(compressor.len(), random_i16s.len());

        let compressor = OffsetCompressor::pack(random_i32s.clone());
        println!("i32 vec: {:?}", random_i32s);
        assert_eq!(compressor.data_size, DataSize::I32);
        assert_eq!(compressor.len(), random_i32s.len());

        let compressor = OffsetCompressor::pack(random_i64s.clone());
        println!("i64 vec: {:?}", random_i64s);
        assert_eq!(compressor.data_size, DataSize::I64);
        assert_eq!(compressor.len(), random_i64s.len());
    }

    #[test]
    fn test_dictionary_compressr_basic_fields() {
        let data = vec![
            i64::MIN,
            i64::MAX,
            i64::MIN,
            i64::MAX,
            i64::MIN,
            i64::MAX,
            i64::MIN,
            i64::MAX,
            i64::MIN,
            i64::MIN,
            i64::MAX,
            i64::MAX,
        ];
        let compressor = DictionaryCompressor::pack(data.clone());
        assert_eq!(compressor.index_size, IndexSize::U8); // the number of unique values is of size i8 (specifically - 2 in this case)
        assert_eq!(compressor.unique_values.len(), 2);
        assert_eq!(
            IndexSize::calculate(compressor.unique_values.len() - 1),
            compressor.index_size
        ); // validating the above statement
        assert_eq!(compressor.len(), data.len())
    }

    #[test]
    fn test_size_compressor_memory_footprint() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let uncompressed_data_memory_footprint =
            size_of_val(&data) + (data.len() * DataSize::I64 as usize);
        let compressor = SizeCompressor::pack(data.clone());
        let compressed_data_memory_footprint = compressor.memory_size();
        println!("data size (stack + heap): {compressed_data_memory_footprint}");
        assert_eq!(
            compressed_data_memory_footprint,
            size_of::<SizeCompressor>() + data.len() * (DataSize::I8 as usize)
        );
        assert!(compressed_data_memory_footprint < uncompressed_data_memory_footprint);
    }

    #[test]
    fn test_offset_compressor_memory_footprint() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let uncompressed_data_memory_footprint =
            size_of_val(&data) + (data.len() * DataSize::I64 as usize);
        let compressor = OffsetCompressor::pack(data.clone());
        let compressed_data_memory_footprint = compressor.memory_size();
        assert_eq!(
            compressed_data_memory_footprint,
            size_of::<OffsetCompressor>() + 10
        );
        assert!(compressed_data_memory_footprint < uncompressed_data_memory_footprint);
    }

    #[test]
    fn test_dictionary_compressor_memory_footprint() {
        let data = vec![
            i64::MIN,
            i64::MAX,
            i64::MIN,
            i64::MAX,
            i64::MIN,
            i64::MAX,
            i64::MIN,
            i64::MAX,
            i64::MIN,
            i64::MIN,
            i64::MAX,
            i64::MAX,
        ];
        let compressor = DictionaryCompressor::pack(data.clone());
        let compressed_data_memory_footprint = compressor.memory_size();
        assert_eq!(
            compressed_data_memory_footprint,
            size_of::<DictionaryCompressor>() + 12 + (2 * DataSize::I64 as usize) as usize
        );
        let uncompressed_data_memory_footprint = data.len() * DataSize::I64 as usize;
        assert!(compressed_data_memory_footprint < uncompressed_data_memory_footprint);
    }

    #[test]
    fn test_compressor_get() {
        let (random_i8s, random_i16s, random_i32s, random_i64s) = generate_random_vecs(2000);

        let i8s_size_compressor = SizeCompressor::pack(random_i8s.clone());
        let i8s_offset_compressor = OffsetCompressor::pack(random_i8s.clone());
        let i8s_dictionary_compressor = DictionaryCompressor::pack(random_i8s.clone());
        for (index, value) in random_i8s.into_iter().enumerate() {
            assert_eq!(i8s_size_compressor.get(index), Some(value));
            assert_eq!(i8s_offset_compressor.get(index), Some(value));
            assert_eq!(i8s_dictionary_compressor.get(index), Some(value));
        }

        let i16s_size_compressor = SizeCompressor::pack(random_i16s.clone());
        let i16s_offset_compressor = OffsetCompressor::pack(random_i16s.clone());
        let i16s_dictionary_compressor = DictionaryCompressor::pack(random_i16s.clone());
        for (index, value) in random_i16s.into_iter().enumerate() {
            assert_eq!(i16s_size_compressor.get(index), Some(value));
            assert_eq!(i16s_offset_compressor.get(index), Some(value));
            assert_eq!(i16s_dictionary_compressor.get(index), Some(value));
        }

        let i32s_size_compressor = SizeCompressor::pack(random_i32s.clone());
        let i32s_offset_compressor = OffsetCompressor::pack(random_i32s.clone());
        let i32s_dictionary_compressor = DictionaryCompressor::pack(random_i32s.clone());
        for (index, value) in random_i32s.into_iter().enumerate() {
            assert_eq!(i32s_size_compressor.get(index), Some(value));
            assert_eq!(i32s_offset_compressor.get(index), Some(value));
            assert_eq!(i32s_dictionary_compressor.get(index), Some(value));
        }

        let i64s_size_compressor = SizeCompressor::pack(random_i64s.clone());
        let i64s_offset_compressor = OffsetCompressor::pack(random_i64s.clone());
        let i64s_dictionary_compressor = DictionaryCompressor::pack(random_i64s.clone());
        for (index, value) in random_i64s.into_iter().enumerate() {
            assert_eq!(i64s_size_compressor.get(index), Some(value));
            assert_eq!(i64s_offset_compressor.get(index), Some(value));
            assert_eq!(i64s_dictionary_compressor.get(index), Some(value));
        }
    }

    #[test]
    fn test_compressed_vector_with_data_for_dictionary_compression() {
        let data_for_dictionary_compressor: Vec<i64> = vec![
            i32::MIN as i64,
            i32::MIN as i64,
            i32::MIN as i64,
            i32::MIN as i64,
            i32::MAX as i64,
            i32::MAX as i64,
            i32::MAX as i64,
            i32::MAX as i64,
        ];
        let compressed_vec = Shrewd::pack(data_for_dictionary_compressor.clone());

        assert!(
            matches!(compressed_vec.0, Inner::Dictionary(_)),
            "Expected Dictionary compressor, but a different strategy was chosen!"
        );

        for index in 0..compressed_vec.len() {
            assert_eq!(
                compressed_vec.get(index),
                Some(data_for_dictionary_compressor[index])
            );
        }
    }

    #[test]
    fn test_compressed_vector_with_data_for_offset_compression() {
        let data_for_offset_compressor: Vec<i64> = vec![
            i64::MAX - 1,
            i64::MAX - 2,
            i64::MAX - 3,
            i64::MAX - 4,
            i64::MAX - 5,
            i64::MAX - 6,
            i64::MAX - 7,
            i64::MAX - 8,
            i64::MAX - 9,
        ];
        let compressed_vec = Shrewd::pack(data_for_offset_compressor.clone());

        assert!(
            matches!(compressed_vec.0, Inner::Offset(_)),
            "Expected Offset compressor, but a different strategy was chosen!"
        );

        for index in 0..compressed_vec.len() {
            assert_eq!(compressed_vec.get(index), Some(data_for_offset_compressor[index]));
        }
    }

    #[test]
    fn test_compressed_vector_with_data_for_size_compression() {
        let data_for_size_compression: Vec<i64> = vec![
            1,2,3,4,5,6,7,8,9,10
        ];

        let compressed_vec = Shrewd::pack(data_for_size_compression.clone());

        assert!(
            matches!(compressed_vec.0, Inner::Size(_)),
            "Expected Size compressor, but a different strategy was chosen!"
        );

        for index in 0..compressed_vec.len() {
            assert_eq!(compressed_vec.get(index), Some(data_for_size_compression[index]));
        }
    }

    #[test]
    fn test_midpoint_and_width() {
        let cases: [(i64, i64, i64, DataSize); 11] = [
            (5, 5, 5, DataSize::I8),
            (-128, 127, 0, DataSize::I8),
            (0, 255, 128, DataSize::I8), // rounding down to 127 would need I16
            (0, 256, 128, DataSize::I16),
            (0, u16::MAX as i64, 32768, DataSize::I16),
            (0, u16::MAX as i64 + 1, 32768, DataSize::I32),
            (0, u32::MAX as i64, 2147483648, DataSize::I32),
            (0, u32::MAX as i64 + 1, 2147483648, DataSize::I64),
            (i64::MIN, i64::MAX, 0, DataSize::I64),
            (i64::MAX - 255, i64::MAX, i64::MAX - 127, DataSize::I8),
            (i64::MIN, i64::MIN + 255, i64::MIN + 128, DataSize::I8),
        ];
        for (min_val, max_val, mid, width) in cases {
            assert_eq!(
                OffsetCompressor::midpoint_and_width(min_val, max_val),
                (mid, width),
                "min={min_val} max={max_val}"
            );
        }
    }

    #[test]
    fn test_offset_compressor_round_trip_at_width_boundaries() {
        let bases = [i64::MIN, -1_000_000_007, -1, 0, 1, 1_000_000_007, i64::MAX];
        let cases = [
            (u8::MAX as u64, DataSize::I8),
            (u8::MAX as u64 + 1, DataSize::I16),
            (u16::MAX as u64, DataSize::I16),
            (u16::MAX as u64 + 1, DataSize::I32),
            (u32::MAX as u64, DataSize::I32),
            (u32::MAX as u64 + 1, DataSize::I64),
        ];
        for base in bases {
            for (range, width) in cases {
                // Keep min..=max inside i64 whichever end the base sits at.
                let min_val = if base > 0 { base.wrapping_sub(range as i64) } else { base };
                let max_val = min_val.wrapping_add(range as i64);
                let data = vec![max_val, min_val, min_val.wrapping_add((range / 2) as i64), max_val];
                let compressor = OffsetCompressor::pack(data.clone());
                assert_eq!(compressor.data_size, width, "min={min_val} max={max_val}");
                for (index, &value) in data.iter().enumerate() {
                    assert_eq!(compressor.get(index), Some(value), "min={min_val} max={max_val}");
                }
            }
        }

        let data = vec![i64::MIN, i64::MAX, 0, -1, 1];
        let compressor = OffsetCompressor::pack(data.clone());
        assert_eq!(compressor.data_size, DataSize::I64);
        for (index, &value) in data.iter().enumerate() {
            assert_eq!(compressor.get(index), Some(value));
        }

        let data = vec![-42; 100];
        let compressor = OffsetCompressor::pack(data.clone());
        assert_eq!(compressor.data_size, DataSize::I8);
        assert_eq!(compressor.len(), data.len());
        for (index, &value) in data.iter().enumerate() {
            assert_eq!(compressor.get(index), Some(value));
        }
    }

    #[test]
    fn test_skewed_data_routes_to_offset() {
        // The mean (~49) put the 250 outlier 201 away, forcing I16; the midpoint keeps every
        // delta within I8.
        let mut data: Vec<i64> = (0..10_000i64).map(|i| (i * 37) % 100).collect();
        data.push(250);
        let compressed_vec = Shrewd::pack(data.clone());
        assert!(
            matches!(compressed_vec.0, Inner::Offset(_)),
            "Expected Offset compressor, but got {}", compressed_vec.compressor_type().as_str()
        );
        assert_eq!(compressed_vec.memory_size(), size_of::<Shrewd>() + data.len());
        for (index, &value) in data.iter().enumerate() {
            assert_eq!(compressed_vec.get(index), Some(value));
        }
    }

    /// Wraps an encoding in a `Shrewd`, so the iterator (which only exists on `Shrewd`) can be
    /// tested on every encoding.
    trait IntoShrewd {
        fn into_shrewd(self) -> Shrewd;
    }

    impl IntoShrewd for Shrewd {
        fn into_shrewd(self) -> Shrewd {
            self
        }
    }

    impl IntoShrewd for SizeCompressor {
        fn into_shrewd(self) -> Shrewd {
            Shrewd(Inner::Size(self))
        }
    }

    impl IntoShrewd for OffsetCompressor {
        fn into_shrewd(self) -> Shrewd {
            Shrewd(Inner::Offset(self))
        }
    }

    impl IntoShrewd for DictionaryCompressor {
        fn into_shrewd(self) -> Shrewd {
            Shrewd(Inner::Dictionary(self))
        }
    }

    /// Encodes `compressor`, decodes the bytes, and checks the copy holds exactly `data`
    /// and re-encodes to the same bytes.
    fn assert_round_trip<C: Compressor + IntoShrewd>(compressor: C, data: &[i64], label: &str) {
        let bytes = compressor.to_bytes();
        let decoded = C::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.len(), data.len(), "{label}: length");
        for (index, &value) in data.iter().enumerate() {
            assert_eq!(decoded.get(index), Some(value), "{label}: index {index}");
        }
        assert_eq!(decoded.get(data.len()), None, "{label}: past the end");
        assert_eq!(decoded.to_bytes(), bytes, "{label}: re-encoded bytes");
        assert_decode_into_matches_get(&decoded, label);
        assert_iter_matches_get(&decoded.into_shrewd(), label);
    }

    #[test]
    fn test_size_compressor_round_trip() {
        let cases: [(Vec<i64>, DataSize); 5] = [
            (vec![], DataSize::I8),
            (vec![i8::MIN as i64, -1, 0, 1, i8::MAX as i64], DataSize::I8),
            (vec![i16::MIN as i64, -300, 0, 1000, i16::MAX as i64], DataSize::I16),
            (vec![i32::MIN as i64, -5_000_000, 0, 5_000_000, i32::MAX as i64], DataSize::I32),
            (vec![i64::MIN, -5_000_000_000, 0, 5_000_000_000, i64::MAX], DataSize::I64),
        ];
        for (data, width) in cases {
            let compressor = SizeCompressor::pack(data.clone());
            assert_eq!(compressor.data_size, width, "{data:?}");
            assert_round_trip(compressor, &data, &format!("Size {width:?}"));
        }
    }

    #[test]
    fn test_offset_compressor_round_trip() {
        let cases: [(Vec<i64>, DataSize); 5] = [
            (vec![], DataSize::I8),
            (vec![1000, 1001, 1100, 1255], DataSize::I8),
            ((0..100).map(|x| 5_000_000 + x * 300).collect(), DataSize::I16),
            ((0..100).map(|x| 10_000_000_000 + x * 10_000_000).collect(), DataSize::I32),
            (vec![i64::MIN, 0, i64::MAX, -1], DataSize::I64),
        ];
        for (data, width) in cases {
            let compressor = OffsetCompressor::pack(data.clone());
            assert_eq!(compressor.data_size, width, "{data:?}");
            assert_round_trip(compressor, &data, &format!("Offset {width:?}"));
        }
    }

    #[test]
    fn test_dictionary_compressor_round_trip() {
        let cases: [(Vec<i64>, IndexSize); 4] = [
            (vec![], IndexSize::U8),
            (vec![7, 7, -7, i64::MAX, i64::MIN, 7], IndexSize::U8),
            ((0..1000).map(|x| (x % 300) << 40).collect(), IndexSize::U16),
            ((0..70_000).map(|x| x * 3).collect(), IndexSize::U32),
        ];
        for (data, width) in cases {
            let compressor = DictionaryCompressor::pack(data.clone());
            assert_eq!(compressor.index_size, width, "{data:?}");
            assert_round_trip(compressor, &data, &format!("Dictionary {width:?}"));
        }
    }

    /// `decode_into` must return exactly what `get` returns, for starts inside, at and past the
    /// end, and for buffers shorter and longer than what remains, without writing past `n`.
    fn assert_decode_into_matches_get<C: Compressor>(compressor: &C, label: &str) {
        // Pre-fills the buffer; any value that never appears in the test data works.
        const UNTOUCHED: i64 = 7_777_777;
        let len = compressor.len();
        let expected: Vec<i64> = (0..len).map(|index| compressor.get(index).unwrap()).collect();
        for start in [0, 1, len / 2, len.saturating_sub(1), len, len + 1] {
            for buffer_len in [0, 1, 7, 64, len + 1] {
                let mut out = vec![UNTOUCHED; buffer_len];
                let n = compressor.decode_into(start, &mut out);
                assert_eq!(n, buffer_len.min(len.saturating_sub(start)), "{label}: count, start {start}, buffer {buffer_len}");
                if n > 0 {
                    assert_eq!(out[..n], expected[start..start + n], "{label}: values, start {start}, buffer {buffer_len}");
                }
                assert!(out[n..].iter().all(|&v| v == UNTOUCHED), "{label}: wrote past n, start {start}, buffer {buffer_len}");
            }
        }
    }

    fn push(mut values: Vec<i64>, value: i64) -> Vec<i64> {
        values.push(value);
        values
    }

    /// `iter` (through `next` and through `fold`) must yield exactly what `get`
    /// returns, report an exact length while being consumed, and stay exhausted at the end.
    fn assert_iter_matches_get(compressor: &Shrewd, label: &str) {
        let expected: Vec<i64> = (0..compressor.len()).map(|index| compressor.get(index).unwrap()).collect();
        assert_eq!(compressor.iter().collect::<Vec<_>>(), expected, "{label}: next");
        assert_eq!(compressor.iter().fold(Vec::new(), push), expected, "{label}: fold");
        assert_eq!(compressor.iter().count(), expected.len(), "{label}: count");
        assert_eq!(compressor.iter_buffered::<1>().collect::<Vec<_>>(), expected, "{label}: next, block 1");
        assert_eq!(compressor.iter_buffered::<1>().fold(Vec::new(), push), expected, "{label}: fold, block 1");
        assert_eq!(compressor.iter_buffered::<1024>().collect::<Vec<_>>(), expected, "{label}: next, block 1024");
        assert_eq!(compressor.iter_buffered::<1024>().fold(Vec::new(), push), expected, "{label}: fold, block 1024");
        assert_eq!(compressor.iter_buffered::<1024>().len(), expected.len(), "{label}: len, block 1024");
        // `sum` would panic on overflow in debug builds, and the test data includes i64::MAX.
        let wrapping_sum = |sum: i64, value: i64| sum.wrapping_add(value);
        assert_eq!(compressor.iter().fold(0, wrapping_sum), expected.iter().copied().fold(0, wrapping_sum), "{label}: sum");

        let mut iter = compressor.iter();
        for consumed in 0..expected.len() {
            assert_eq!(iter.len(), expected.len() - consumed, "{label}: len after {consumed}");
            iter.next();
        }
        assert_eq!(iter.len(), 0, "{label}: len at the end");
        assert_eq!(iter.next(), None, "{label}: exhausted");
        assert_eq!(iter.next(), None, "{label}: stays exhausted");

        // `fold` after some `next` calls must first hand out what is left in the buffer.
        for taken in [1, 3, BLOCK_SIZE - 1, BLOCK_SIZE, BLOCK_SIZE + 1] {
            let mut iter = compressor.iter();
            let mut values: Vec<i64> = iter.by_ref().take(taken).collect();
            values = iter.fold(values, push);
            assert_eq!(values, expected, "{label}: fold after {taken} next calls");
        }
    }

    #[test]
    fn test_iter_block_boundaries() {
        for len in [0, 1, BLOCK_SIZE - 1, BLOCK_SIZE, BLOCK_SIZE + 1, 2 * BLOCK_SIZE + 1] {
            let data: Vec<i64> = (0..len as i64).map(|x| x * 7 - 100).collect();
            let compressor = SizeCompressor::pack(data.clone()).into_shrewd();
            assert_eq!(compressor.iter().collect::<Vec<_>>(), data, "Size len {len}");
            assert_iter_matches_get(&compressor, &format!("Size len {len}"));
            assert_iter_matches_get(&Shrewd::pack(data), &format!("Shrewd len {len}"));
        }
    }

    /// Every strict prefix of a valid encoding must fail to decode instead of panicking.
    fn assert_truncation_is_eof<C: Compressor>(compressor: C, label: &str) {
        let bytes = compressor.to_bytes();
        for len in 0..bytes.len() {
            assert_eq!(C::from_bytes(&bytes[..len]).err(), Some(DecodeError::UnexpectedEOF), "{label}: truncated to {len}");
        }
    }

    #[test]
    fn test_truncated_bytes_are_eof() {
        let data: Vec<i64> = vec![7, 7, -7, 300, 7];
        assert_truncation_is_eof(SizeCompressor::pack(data.clone()), "Size");
        assert_truncation_is_eof(OffsetCompressor::pack(data.clone()), "Offset");
        assert_truncation_is_eof(DictionaryCompressor::pack(data.clone()), "Dictionary");
        assert_truncation_is_eof(Shrewd::pack(data), "Shrewd");
    }

    #[test]
    fn test_wrong_header_is_rejected() {
        let bytes = SizeCompressor::pack(vec![1, 2, 3]).to_bytes();
        assert_eq!(OffsetCompressor::from_bytes(&bytes).err(), Some(DecodeError::InvalidCompressorType(CompressorType::Size as u8)));
        assert_eq!(Shrewd::from_bytes(&[FORMAT_VERSION, 9]).err(), Some(DecodeError::InvalidCompressorType(9)));
    }

    #[test]
    fn test_unknown_version_is_rejected() {
        let mut bytes = Shrewd::pack(vec![1, 2, 3]).to_bytes();
        bytes[0] = FORMAT_VERSION + 1;
        assert_eq!(Shrewd::from_bytes(&bytes).err(), Some(DecodeError::UnsupportedVersion(FORMAT_VERSION + 1)));
        assert_eq!(SizeCompressor::from_bytes(&bytes).err(), Some(DecodeError::UnsupportedVersion(FORMAT_VERSION + 1)));
    }

    /// The byte layout is a public format: these exact bytes must not change without changing
    /// FORMAT_VERSION. Every value is spelled out by hand rather than produced by the encoder.
    #[test]
    fn test_byte_format_is_fixed() {
        let size = [
            1, 1, // version, Size
            1, // width: 1 byte
            3, 0, 0, 0, 0, 0, 0, 0, // length: 3 (u64)
            1, 0xFE, 3, // 1, -2, 3
        ];
        assert_eq!(SizeCompressor::pack(vec![1, -2, 3]).to_bytes(), size);

        let offset = [
            1, 2, // version, Offset
            1, // width: 1 byte
            3, 0, 0, 0, 0, 0, 0, 0, // length: 3 (u64)
            0xE9, 0x03, 0, 0, 0, 0, 0, 0, // center: 1001 (i64)
            0xFF, 0, 1, // deltas: -1, 0, 1
        ];
        assert_eq!(OffsetCompressor::pack(vec![1000, 1001, 1002]).to_bytes(), offset);

        let dictionary = [
            1, 3, // version, Dictionary
            1, // index width: 1 byte
            2, 0, 0, 0, 0, 0, 0, 0, // table length: 2 (u64)
            7, 0, 0, 0, 0, 0, 0, 0, // table[0]: 7 (i64)
            0xF9, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // table[1]: -7 (i64)
            3, 0, 0, 0, 0, 0, 0, 0, // length: 3 (u64)
            0, 1, 0, // indices
        ];
        assert_eq!(DictionaryCompressor::pack(vec![7, -7, 7]).to_bytes(), dictionary);
    }

    #[test]
    fn test_dictionary_index_out_of_range_is_rejected() {
        let mut bytes = DictionaryCompressor::pack(vec![7, -7, 7]).to_bytes();
        let last = bytes.len() - 1;
        bytes[last] = 5; // the table has 2 entries
        assert_eq!(DictionaryCompressor::from_bytes(&bytes).err(), Some(DecodeError::InvalidDictionaryIndex(5)));
        assert_eq!(Shrewd::from_bytes(&bytes).err(), Some(DecodeError::InvalidDictionaryIndex(5)));

        // Indices wider than one byte are checked too.
        let wide = DictionaryCompressor::pack((0..300).collect());
        assert_eq!(wide.index_size, IndexSize::U16);
        let mut bytes = wide.to_bytes();
        let last = bytes.len() - 2;
        bytes[last..].copy_from_slice(&300u16.to_le_bytes());
        assert_eq!(DictionaryCompressor::from_bytes(&bytes).err(), Some(DecodeError::InvalidDictionaryIndex(300)));
    }

    #[test]
    fn test_shrewd_round_trip() {
        let size_data: Vec<i64> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let offset_data: Vec<i64> = (0..100).map(|x| 5_000_000 + x * 300).collect();
        let dictionary_data: Vec<i64> = (0..1000).map(|x| (x % 300) << 40).collect();

        let compressor = Shrewd::pack(size_data.clone());
        assert!(matches!(compressor.0, Inner::Size(_)));
        assert_round_trip(compressor, &size_data, "Shrewd Size");

        let compressor = Shrewd::pack(offset_data.clone());
        assert!(matches!(compressor.0, Inner::Offset(_)));
        assert_round_trip(compressor, &offset_data, "Shrewd Offset");

        let compressor = Shrewd::pack(dictionary_data.clone());
        assert!(matches!(compressor.0, Inner::Dictionary(_)));
        assert_round_trip(compressor, &dictionary_data, "Shrewd Dictionary");

        assert_round_trip(Shrewd::pack(vec![]), &[], "Shrewd empty");
    }
}