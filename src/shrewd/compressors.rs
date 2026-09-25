use std::mem::{size_of, size_of_val};
use std::collections::{HashMap};

use super::sizes::{HyperLogLog, DataSize, IndexSize};
use super::{Compressor, Shrewd, SizeCompressor, AverageCompressor, DictionaryCompressor};


impl Compressor for Shrewd {
    fn pack(v: Vec<i64>) -> Self {
        if v.is_empty() {
            return Shrewd::Size(SizeCompressor::pack(v));
        }

        // --- PHASE 1: LIGHTWEIGHT ESTIMATION PASS (O(N)) ---
        let mut min_val = v[0];
        let mut max_val = v[0];
        let mut avg: f64 = 0.0f64;
        let mut original_data_size = DataSize::I8;

        let mut hll = HyperLogLog::new(12);

        for (index, &value) in v.iter().enumerate() {
            if value < min_val {
                min_val = value;
            }
            if value > max_val {
                max_val = value;
            }
            let count = (index + 1) as f64;
            avg += (value as f64 - avg) / count;

            let val_size = DataSize::calculate(value);
            if val_size > original_data_size {
                original_data_size = val_size;
            }

            hll.insert(value);
        }

        let avg = avg.round() as i64;

        // --- PHASE 2: CALCULATE PRECISE METRIC FOOTPRINTS ---
        // 1. SizeCompressor Footprint
        let size_est_bytes = v.len() * original_data_size as u8 as usize;

        // 2. AverageCompressor Footprint
        let mut max_delta_from_avg = DataSize::I8;
        for &val in v.iter() {
            let delta = match original_data_size {
                DataSize::I8 => (val as i8).wrapping_sub(avg as i8) as i64,
                DataSize::I16 => (val as i16).wrapping_sub(avg as i16) as i64,
                DataSize::I32 => (val as i32).wrapping_sub(avg as i32) as i64,
                DataSize::I64 => val.wrapping_sub(avg),
            };
            let d = DataSize::calculate(delta);
            if d > max_delta_from_avg {
                max_delta_from_avg = d;
                if max_delta_from_avg == DataSize::I64 { break; }
            }
        }
        let avg_est_bytes = v.len() * max_delta_from_avg as u8 as usize;

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

        if low_uniqueness && dict_est_bytes < size_est_bytes && dict_est_bytes < avg_est_bytes {
            Shrewd::Dictionary(DictionaryCompressor::pack(v))
        } else if avg_est_bytes < size_est_bytes {
            Shrewd::Average(AverageCompressor::_compress(v, original_data_size, avg, max_delta_from_avg))
        } else {
            Shrewd::Size(SizeCompressor::_compress(v, original_data_size))
        }
    }

    #[inline(always)]
    fn get(&self, index: usize) -> i64 {
        match self {
            Shrewd::Size(c) => c.get(index),
            Shrewd::Average(c) => c.get(index),
            Shrewd::Dictionary(c) => c.get(index),
        }
    }

    #[inline]
    fn size(&self) -> usize {
        // Includes the small stack-overhead variant size of the tracking enum container itself
        match self {
            Shrewd::Size(c) => c.size() + size_of_val(self),
            Shrewd::Average(c) => c.size() + size_of_val(self),
            Shrewd::Dictionary(c) => c.size() + size_of_val(self),
        }
    }

    #[inline]
    fn length(&self) -> usize {
        match self {
            Shrewd::Size(c) => c.length(),
            Shrewd::Average(c) => c.length(),
            Shrewd::Dictionary(c) => c.length(),
        }
    }

    #[inline(always)]
    fn compressor_type(&self) -> String {
        match self {
            Shrewd::Size(c) => c.compressor_type(),
            Shrewd::Average(c) => c.compressor_type(),
            Shrewd::Dictionary(c) => c.compressor_type(),
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

    fn get(&self, index: usize) -> i64 {
        match self.data_size {
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
        }
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
        String::from("Size")
    }
}



impl AverageCompressor {
    pub(crate) fn _compress(v: Vec<i64>, original_data_size: DataSize, avg: i64, max_delta_from_avg: DataSize) -> Self {
        let cap;
        match max_delta_from_avg {
            DataSize::I8 => cap = v.len(),
            DataSize::I16 => cap = size_of::<i16>() * v.len(),
            DataSize::I32 => cap = size_of::<i32>() * v.len(),
            DataSize::I64 => cap = size_of::<i64>() * v.len(),
        }

        let mut compressed_data: Vec<u8> = Vec::with_capacity(cap);

        for value in v.into_iter() {
            let transformed_value = match original_data_size {
                DataSize::I8 => (value as i8).wrapping_sub(avg as i8) as i64,
                DataSize::I16 => (value as i16).wrapping_sub(avg as i16) as i64,
                DataSize::I32 => (value as i32).wrapping_sub(avg as i32) as i64,
                DataSize::I64 => value.wrapping_sub(avg),
            };
            match max_delta_from_avg {
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

        AverageCompressor {
            data: compressed_data,
            data_size: max_delta_from_avg,
            avg: avg,
            original_data_size: original_data_size,
        }
    }
}

impl Compressor for AverageCompressor {
    fn pack(v: Vec<i64>) -> Self {
        let mut avg: f64 = 0f64;
        let mut original_data_size = DataSize::I8;
        for (index, &value) in v.iter().enumerate() {
            let count = (index + 1) as f64;
            avg += (value as f64 - avg) / count;
            let data_size = DataSize::calculate(value);
            if data_size > original_data_size {
                original_data_size = data_size;
            }
        }
        let avg = avg.round() as i64;
        let mut size = DataSize::I8;

        for &value in v.iter() {
            let data_size = match original_data_size {
                DataSize::I8 => DataSize::calculate((value as i8).wrapping_sub(avg as i8) as i64),
                DataSize::I16 => {
                    DataSize::calculate((value as i16).wrapping_sub(avg as i16) as i64)
                }
                DataSize::I32 => {
                    DataSize::calculate((value as i32).wrapping_sub(avg as i32) as i64)
                }
                DataSize::I64 => DataSize::calculate(value.wrapping_sub(avg)),
            };
            if data_size > size {
                size = data_size;
                if size == DataSize::I64 {
                    break;
                }
            }
        }
        Self::_compress(v, original_data_size, avg, size)
    }

    fn get(&self, index: usize) -> i64 {
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

        match self.original_data_size {
            // no bugs here because the avg is at most the size of the self.original_data_size field
            DataSize::I8 => (transformed_value as i8).wrapping_add(self.avg as i8) as i64,
            DataSize::I16 => (transformed_value as i16).wrapping_add(self.avg as i16) as i64,
            DataSize::I32 => (transformed_value as i32).wrapping_add(self.avg as i32) as i64,
            DataSize::I64 => transformed_value.wrapping_add(self.avg),
        }
    }

    fn size(&self) -> usize {
        // size of vec data + vec pointer + size enum + avg
        self.data.len() + size_of_val(self)
    }

    #[inline]
    fn length(&self) -> usize {
        self.data.len() >> self.data_size.shift_factor()
    }

    #[inline(always)]
    fn compressor_type(&self) -> String {
        String::from("Average")
    }
}

impl Compressor for DictionaryCompressor {
    fn pack(v: Vec<i64>) -> Self {
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

    fn get(&self, index: usize) -> i64 {
        match self.index_size {
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
        }
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
        String::from("Dictionary")
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
    fn test_avg_compressor_basic_fields() {
        let (random_i8s, random_i16s, random_i32s, random_i64s) = generate_random_vecs(10);

        let compressor = AverageCompressor::pack(random_i8s.clone());
        println!("i8 vec: {:?}", random_i8s);
        assert_eq!(compressor.data_size, DataSize::I8);
        assert_eq!(compressor.length(), random_i8s.len());

        let compressor = AverageCompressor::pack(random_i16s.clone());
        println!("i16 vec: {:?}", random_i16s);
        assert_eq!(compressor.data_size, DataSize::I16);
        assert_eq!(compressor.length(), random_i16s.len());

        let compressor = AverageCompressor::pack(random_i32s.clone());
        println!("i32 vec: {:?}", random_i32s);
        assert_eq!(compressor.data_size, DataSize::I32);
        assert_eq!(compressor.length(), random_i32s.len());

        let compressor = AverageCompressor::pack(random_i64s.clone());
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
    fn test_avg_compressor_memory_footprint() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let uncompressed_data_memory_footprint =
            size_of_val(&data) + (data.len() * DataSize::I64 as usize);
        let compressor = AverageCompressor::pack(data.clone());
        let compressed_data_memory_footprint = compressor.size();
        assert_eq!(
            compressed_data_memory_footprint,
            size_of::<AverageCompressor>() + 10
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
        let i8s_avg_compressor = AverageCompressor::pack(random_i8s.clone());
        let i8s_dictionary_compressor = DictionaryCompressor::pack(random_i8s.clone());
        for (index, value) in random_i8s.into_iter().enumerate() {
            assert_eq!(i8s_size_compressor.get(index), value);
            assert_eq!(i8s_avg_compressor.get(index), value);
            assert_eq!(i8s_dictionary_compressor.get(index), value);
        }

        let i16s_size_compressor = SizeCompressor::pack(random_i16s.clone());
        let i16s_avg_compressor = AverageCompressor::pack(random_i16s.clone());
        let i16s_dictionary_compressor = DictionaryCompressor::pack(random_i16s.clone());
        for (index, value) in random_i16s.into_iter().enumerate() {
            assert_eq!(i16s_size_compressor.get(index), value);
            assert_eq!(i16s_avg_compressor.get(index), value);
            assert_eq!(i16s_dictionary_compressor.get(index), value);
        }

        let i32s_size_compressor = SizeCompressor::pack(random_i32s.clone());
        let i32s_avg_compressor = AverageCompressor::pack(random_i32s.clone());
        let i32s_dictionary_compressor = DictionaryCompressor::pack(random_i32s.clone());
        for (index, value) in random_i32s.into_iter().enumerate() {
            assert_eq!(i32s_size_compressor.get(index), value);
            assert_eq!(i32s_avg_compressor.get(index), value);
            assert_eq!(i32s_dictionary_compressor.get(index), value);
        }

        let i64s_size_compressor = SizeCompressor::pack(random_i64s.clone());
        let i64s_avg_compressor = AverageCompressor::pack(random_i64s.clone());
        let i64s_dictionary_compressor = DictionaryCompressor::pack(random_i64s.clone());
        for (index, value) in random_i64s.into_iter().enumerate() {
            assert_eq!(i64s_size_compressor.get(index), value);
            assert_eq!(i64s_avg_compressor.get(index), value);
            assert_eq!(i64s_dictionary_compressor.get(index), value);
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
            matches!(compressed_vec, Shrewd::Dictionary(_)),
            "Expected Dictionary compressor, but a different strategy was chosen!"
        );

        for index in 0..compressed_vec.length() {
            assert_eq!(
                compressed_vec.get(index),
                data_for_dictionary_compressor[index]
            );
        }
    }

    #[test]
    fn test_compressed_vector_with_data_for_average_compression() {
        let data_for_avg_compressor: Vec<i64> = vec![
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
        let compressed_vec = Shrewd::pack(data_for_avg_compressor.clone());

        assert!(
            matches!(compressed_vec, Shrewd::Average(_)),
            "Expected Average compressor, but a different strategy was chosen!"
        );

        for index in 0..compressed_vec.length() {
            assert_eq!(compressed_vec.get(index), data_for_avg_compressor[index]);
        }
    }

    #[test]
    fn test_compressed_vector_with_data_for_size_compression() {
        let data_for_size_compression: Vec<i64> = vec![
            1,2,3,4,5,6,7,8,9,10
        ];

        let compressed_vec = Shrewd::pack(data_for_size_compression.clone());

        assert!(
            matches!(compressed_vec, Shrewd::Size(_)),
            "Expected Average compressor, but a different strategy was chosen!"
        );

        for index in 0..compressed_vec.length() {
            assert_eq!(compressed_vec.get(index), data_for_size_compression[index]);
        }
    }
}