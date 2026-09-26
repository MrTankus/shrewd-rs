use std::cmp::max;
use std::mem::{size_of, size_of_val};
use std::collections::{HashMap};
use std::string::ToString;
use super::sizes::{HyperLogLog, DataSize, IndexSize};
use super::{Compressor, Shrewd, Inner, SizeCompressor, OffsetCompressor, DictionaryCompressor};

const COMPRESSOR_TYPE_SIZE: &str = "Size";
const COMPRESSOR_TYPE_OFFSET: &str = "Offset";
const COMPRESSOR_TYPE_DICTIONARY: &str = "Dictionary";



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
        let unique_count = hll.estimate(); // Highly scalable O(1) extraction
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
            Shrewd(Inner::Dictionary(DictionaryCompressor::pack(v)))
        } else if offset_est_bytes < size_est_bytes {
            Shrewd(Inner::Offset(OffsetCompressor::_compress(v, original_data_size, center, max_delta_from_center)))
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
    fn size(&self) -> usize {
        // Includes the small stack-overhead variant size of the tracking enum container itself
        match &self.0 {
            Inner::Size(c) => c.size() - size_of_val(c) + size_of_val(self),
            Inner::Offset(c) => c.size() - size_of_val(c) + size_of_val(self),
            Inner::Dictionary(c) => c.size() - size_of_val(c) + size_of_val(self),
        }
    }

    #[inline]
    fn length(&self) -> usize {
        match &self.0 {
            Inner::Size(c) => c.length(),
            Inner::Offset(c) => c.length(),
            Inner::Dictionary(c) => c.length(),
        }
    }

    #[inline(always)]
    fn compressor_type(&self) -> String {
        match &self.0 {
            Inner::Size(c) => c.compressor_type(),
            Inner::Offset(c) => c.compressor_type(),
            Inner::Dictionary(c) => c.compressor_type(),
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
                    data.extend_from_slice(&(value as i16).to_ne_bytes());
                }
            }
            DataSize::I32 => {
                for value in v {
                    data.extend_from_slice(&(value as i32).to_ne_bytes());
                }
            }
            DataSize::I64 => {
                for value in v {
                    data.extend_from_slice(&value.to_ne_bytes());
                }
            }
        }

        SizeCompressor {
            data: data,
            data_size: data_size,
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
        if index >= self.length() {
            return None;
        }
        let value = match self.data_size {
            DataSize::I8 => self.data[index] as i8 as i64,
            DataSize::I16 => {
                let real_index = index * DataSize::I16 as usize;
                let val = i16::from_ne_bytes(
                    self.data[real_index..(real_index + DataSize::I16 as usize)]
                        .try_into()
                        .unwrap(),
                );
                val as i64
            }
            DataSize::I32 => {
                let real_index = index * DataSize::I32 as usize;
                let val = i32::from_ne_bytes(
                    self.data[real_index..(real_index + DataSize::I32 as usize)]
                        .try_into()
                        .unwrap(),
                );
                val as i64
            }
            DataSize::I64 => {
                let real_index = index * DataSize::I64 as usize;
                i64::from_ne_bytes(
                    self.data[real_index..(real_index + DataSize::I64 as usize)]
                        .try_into()
                        .unwrap(),
                )
            }
        };
        Some(value)
    }

    fn size(&self) -> usize {
        self.data.len() + size_of_val(self)
    }

    #[inline]
    fn length(&self) -> usize {
        self.data.len() >> self.data_size.shift_factor()
    }

    #[inline(always)]
    fn compressor_type(&self) -> String {
        COMPRESSOR_TYPE_SIZE.to_string()
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

    pub(crate) fn _compress(v: Vec<i64>, original_data_size: DataSize, center: i64, max_delta_from_center: DataSize) -> Self {
        let cap;
        match max_delta_from_center {
            DataSize::I8 => cap = v.len(),
            DataSize::I16 => cap = size_of::<i16>() * v.len(),
            DataSize::I32 => cap = size_of::<i32>() * v.len(),
            DataSize::I64 => cap = size_of::<i64>() * v.len(),
        }

        let mut compressed_data: Vec<u8> = Vec::with_capacity(cap);

        for value in v.into_iter() {
            let transformed_value = match original_data_size {
                DataSize::I8 => (value as i8).wrapping_sub(center as i8) as i64,
                DataSize::I16 => (value as i16).wrapping_sub(center as i16) as i64,
                DataSize::I32 => (value as i32).wrapping_sub(center as i32) as i64,
                DataSize::I64 => value.wrapping_sub(center),
            };
            match max_delta_from_center {
                DataSize::I8 => {
                    compressed_data.extend_from_slice(&(transformed_value as i8).to_ne_bytes());
                }
                DataSize::I16 => {
                    compressed_data.extend_from_slice(&(transformed_value as i16).to_ne_bytes());
                }
                DataSize::I32 => {
                    compressed_data.extend_from_slice(&(transformed_value as i32).to_ne_bytes());
                }
                DataSize::I64 => {
                    compressed_data.extend_from_slice(&transformed_value.to_ne_bytes());
                }
            }
        }

        OffsetCompressor {
            data: compressed_data,
            data_size: max_delta_from_center,
            center: center,
            original_data_size: original_data_size,
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
                original_data_size: DataSize::I8,
            };
        }
        let mut min_val = i64::MAX;
        let mut max_val = i64::MIN;
        for &value in v.iter() {
            min_val = min_val.min(value);
            max_val = max_val.max(value);
        }
        let original_data_size = max(DataSize::calculate(min_val), DataSize::calculate(max_val));
        let (center, size) = Self::midpoint_and_width(min_val, max_val);
        Self::_compress(v, original_data_size, center, size)
    }

    fn get(&self, index: usize) -> Option<i64> {
        if index >= self.length() {
            return None;
        }
        let transformed_value = match self.data_size {
            DataSize::I8 => self.data[index] as i8 as i64,
            DataSize::I16 => {
                let real_index = index * DataSize::I16 as usize;
                i16::from_ne_bytes(
                    self.data[real_index..(real_index + DataSize::I16 as usize)]
                        .try_into()
                        .unwrap(),
                ) as i64
            }
            DataSize::I32 => {
                let real_index = index * DataSize::I32 as usize;
                i32::from_ne_bytes(
                    self.data[real_index..(real_index + DataSize::I32 as usize)]
                        .try_into()
                        .unwrap(),
                ) as i64
            }
            DataSize::I64 => {
                let real_index = index * DataSize::I64 as usize;
                i64::from_ne_bytes(
                    self.data[real_index..(real_index + DataSize::I64 as usize)]
                        .try_into()
                        .expect("Slice was not exactly 8 bytes long"),
                )
            }
        };

        let value = match self.original_data_size {
            // no bugs here because the center is at most the size of the self.original_data_size field
            DataSize::I8 => (transformed_value as i8).wrapping_add(self.center as i8) as i64,
            DataSize::I16 => (transformed_value as i16).wrapping_add(self.center as i16) as i64,
            DataSize::I32 => (transformed_value as i32).wrapping_add(self.center as i32) as i64,
            DataSize::I64 => transformed_value.wrapping_add(self.center),
        };
        Some(value)
    }

    fn size(&self) -> usize {
        // size of vec data + vec pointer + size enum + center
        self.data.len() + size_of_val(self)
    }

    #[inline]
    fn length(&self) -> usize {
        self.data.len() >> self.data_size.shift_factor()
    }

    #[inline(always)]
    fn compressor_type(&self) -> String {
        COMPRESSOR_TYPE_OFFSET.to_string()
    }
}

impl Compressor for DictionaryCompressor {
    fn pack(v: Vec<i64>) -> Self {
        if v.is_empty() {
            return DictionaryCompressor {
                data: vec![],
                unique_values: vec![],
                index_size: IndexSize::U8,
            };
        }
        let mut lookup: HashMap<i64, usize> = HashMap::with_capacity(v.len() / 2);
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
                    data.extend_from_slice(&((index as u16).to_ne_bytes()));
                }
            }
            IndexSize::U32 => {
                for value in v.into_iter() {
                    let index = lookup[&value];
                    data.extend_from_slice(&((index as u32).to_ne_bytes()));
                }
            }
            IndexSize::U64 => {
                for value in v.into_iter() {
                    let index = lookup[&value];
                    data.extend_from_slice(&((index as u64).to_ne_bytes()));
                }
            }
        }

        DictionaryCompressor {
            data: data,
            unique_values: unique_values,
            index_size: data_size,
        }
    }

    fn get(&self, index: usize) -> Option<i64> {
        if index >= self.length() {
            return None;
        }
        let value = match self.index_size {
            IndexSize::U8 => {
                let dict_index = self.data[index];
                self.unique_values[dict_index as usize]
            }
            IndexSize::U16 => {
                let real_index = index * IndexSize::U16 as usize as usize;
                let dict_index: u16 = u16::from_ne_bytes(
                    self.data[real_index..(real_index + IndexSize::U16 as usize)]
                        .try_into()
                        .expect("Slice was not exactly 2 bytes long"),
                );
                self.unique_values[dict_index as usize]
            }
            IndexSize::U32 => {
                let real_index = index * IndexSize::U32 as usize as usize;
                let dict_index: u32 = u32::from_ne_bytes(
                    self.data[real_index..(real_index + IndexSize::U32 as usize)]
                        .try_into()
                        .expect("Slice was not exactly 4 bytes long"),
                );
                self.unique_values[dict_index as usize]
            }
            IndexSize::U64 => {
                let real_index = index * IndexSize::U64 as usize;
                let dict_index: u64 = u64::from_ne_bytes(
                    self.data[real_index..(real_index + IndexSize::U64 as usize)]
                        .try_into()
                        .expect("Slice was not exactly 8 bytes long"),
                );
                self.unique_values[dict_index as usize]
            }
        };
        Some(value)
    }

    #[inline]
    fn length(&self) -> usize {
        self.data.len() >> self.index_size.shift_factor()
    }

    fn size(&self) -> usize {
        self.data.len() + (self.unique_values.len() * DataSize::I64 as usize) + size_of_val(self)
    }

    #[inline(always)]
    fn compressor_type(&self) -> String {
        COMPRESSOR_TYPE_DICTIONARY.to_string()
    }
}

#[cfg(test)]
mod tests {

    use super::*;
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
        assert_eq!(compressor.length(), random_i8s.len());
        assert_eq!(compressor.size(), size_of_val(&compressor) + (random_i8s.len() * DataSize::I8 as usize));

        let compressor = SizeCompressor::pack(random_i16s.clone());
        assert_eq!(compressor.data_size, DataSize::I16);
        assert_eq!(compressor.length(), random_i16s.len());
        assert_eq!(compressor.size(), size_of_val(&compressor) + (random_i16s.len() * DataSize::I16 as usize));

        let compressor = SizeCompressor::pack(random_i32s.clone());
        assert_eq!(compressor.data_size, DataSize::I32);
        assert_eq!(compressor.length(), random_i32s.len());
        assert_eq!(compressor.size(), size_of_val(&compressor) + (random_i32s.len() * DataSize::I32 as usize));

        let compressor = SizeCompressor::pack(random_i64s.clone());
        assert_eq!(compressor.data_size, DataSize::I64);
        assert_eq!(compressor.length(), random_i64s.len());
        assert_eq!(compressor.size(), size_of_val(&compressor) + (random_i64s.len() * DataSize::I64 as usize));
    }

    #[test]
    fn test_offset_compressor_basic_fields() {
        let (random_i8s, random_i16s, random_i32s, random_i64s) = generate_random_vecs(10);

        let compressor = OffsetCompressor::pack(random_i8s.clone());
        println!("i8 vec: {:?}", random_i8s);
        assert_eq!(compressor.data_size, DataSize::I8);
        assert_eq!(compressor.length(), random_i8s.len());

        let compressor = OffsetCompressor::pack(random_i16s.clone());
        println!("i16 vec: {:?}", random_i16s);
        assert_eq!(compressor.data_size, DataSize::I16);
        assert_eq!(compressor.length(), random_i16s.len());

        let compressor = OffsetCompressor::pack(random_i32s.clone());
        println!("i32 vec: {:?}", random_i32s);
        assert_eq!(compressor.data_size, DataSize::I32);
        assert_eq!(compressor.length(), random_i32s.len());

        let compressor = OffsetCompressor::pack(random_i64s.clone());
        println!("i64 vec: {:?}", random_i64s);
        assert_eq!(compressor.data_size, DataSize::I64);
        assert_eq!(compressor.length(), random_i64s.len());
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
        assert_eq!(compressor.length(), data.len())
    }

    #[test]
    fn test_size_compressor_memory_footprint() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let uncompressed_data_memory_footprint =
            size_of_val(&data) + (data.len() * DataSize::I64 as usize);
        let compressor = SizeCompressor::pack(data.clone());
        let compressed_data_memory_footprint = compressor.size();
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
        let compressed_data_memory_footprint = compressor.size();
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
        let compressed_data_memory_footprint = compressor.size();
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

        for index in 0..compressed_vec.length() {
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

        for index in 0..compressed_vec.length() {
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

        for index in 0..compressed_vec.length() {
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
        assert_eq!(compressor.length(), data.len());
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
            "Expected Offset compressor, but got {}", compressed_vec.compressor_type()
        );
        assert_eq!(compressed_vec.size(), size_of::<Shrewd>() + data.len());
        for (index, &value) in data.iter().enumerate() {
            assert_eq!(compressed_vec.get(index), Some(value));
        }
    }
}