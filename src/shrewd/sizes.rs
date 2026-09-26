
use std::mem::size_of;

pub(crate) const SIZE_I8: u8 = size_of::<i8>() as u8;
pub(crate) const SIZE_I16: u8 = size_of::<i16>() as u8;
pub(crate) const SIZE_I32: u8 = size_of::<i32>() as u8;
pub(crate) const SIZE_I64: u8 = size_of::<i64>() as u8;


#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
#[repr(u8)]
pub(crate) enum DataSize {
    I8 = SIZE_I8,
    I16 = SIZE_I16,
    I32 = SIZE_I32,
    I64 = SIZE_I64,
}

impl DataSize {
    pub(crate) fn calculate(value: i64) -> DataSize {
        if value >= i8::MIN as i64 && value <= i8::MAX as i64 {
            DataSize::I8
        } else if value >= i16::MIN as i64 && value <= i16::MAX as i64 {
            DataSize::I16
        } else if value >= i32::MIN as i64 && value <= i32::MAX as i64 {
            DataSize::I32
        } else {
            DataSize::I64
        }
    }

    #[inline(always)]
    pub(crate) fn shift_factor(self) -> usize {
        match self {
            DataSize::I8 => 0,
            DataSize::I16 => 1,
            DataSize::I32 => 2,
            DataSize::I64 => 3,
        }
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
#[repr(u8)]
pub(crate) enum IndexSize {
    U8 = SIZE_I8,
    U16 = SIZE_I16,
    U32 = SIZE_I32,
    U64 = SIZE_I64,
}

impl IndexSize {
    pub(crate) fn calculate(value: usize) -> IndexSize {
        if value <= u8::MAX as usize {
            IndexSize::U8
        } else if value <= u16::MAX as usize {
            IndexSize::U16
        } else if value <= u32::MAX as usize {
            IndexSize::U32
        } else {
            IndexSize::U64
        }
    }

    #[inline(always)]
    pub(crate) fn shift_factor(self) -> usize {
        match self {
            IndexSize::U8 => 0,
            IndexSize::U16 => 1,
            IndexSize::U32 => 2,
            IndexSize::U64 => 3,
        }
    }
}

pub(crate) struct HyperLogLog {
    registers: Vec<u8>,
    precision: u32, // bits used to select register; m = 2^precision registers
}

impl HyperLogLog {
    pub(crate) fn new(precision: u32) -> Self {
        HyperLogLog { registers: vec![0u8; 1 << precision], precision }
    }

    #[inline]
    pub(crate) fn insert(&mut self, value: i64) {
        let hash = Self::mix(value as u64);
        let idx = (hash & ((1 << self.precision) - 1)) as usize;
        let rest = hash >> self.precision;
        let rank = (rest.trailing_zeros() + 1) as u8;
        if rank > self.registers[idx] {
            self.registers[idx] = rank;
        }
    }

    pub(crate) fn estimate(&self) -> usize {
        let m = self.registers.len() as f64;
        let alpha = 0.7213 / (1.0 + 1.079 / m);
        let sum: f64 = self.registers.iter().map(|&r| 2f64.powi(-(r as i32))).sum();
        let raw = alpha * m * m / sum;

        // Small-range correction (linear counting) — matters when cardinality << m
        let final_est = if raw <= 2.5 * m {
            let zeros = self.registers.iter().filter(|&&r| r == 0).count();
            if zeros != 0 {
                m * (m / zeros as f64).ln()
            } else {
                raw
            }
        } else {
            raw
        };
        (final_est.round() as usize).max(1)
    }

    // splitmix64 finalizer — cheap, well-distributed, no external dependency
    #[inline]
    fn mix(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9E3779B97F4A7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_calculate_data_size() {
        assert_eq!(DataSize::calculate(0), DataSize::I8);
        assert_eq!(DataSize::calculate(-100), DataSize::I8);
        assert_eq!(DataSize::calculate(100), DataSize::I8);
        assert_eq!(DataSize::calculate(i8::MIN as i64), DataSize::I8);
        assert_eq!(DataSize::calculate(i8::MAX as i64), DataSize::I8);

        assert_eq!(DataSize::calculate(-500), DataSize::I16);
        assert_eq!(DataSize::calculate(500), DataSize::I16);
        assert_eq!(DataSize::calculate(i16::MIN as i64), DataSize::I16);
        assert_eq!(DataSize::calculate(i16::MAX as i64), DataSize::I16);

        assert_eq!(DataSize::calculate(5000000), DataSize::I32);
        assert_eq!(DataSize::calculate(-5000000), DataSize::I32);
        assert_eq!(DataSize::calculate(i32::MIN as i64), DataSize::I32);
        assert_eq!(DataSize::calculate(i32::MAX as i64), DataSize::I32);

        assert_eq!(DataSize::calculate(5000000000), DataSize::I64);
        assert_eq!(DataSize::calculate(-5000000000), DataSize::I64);
        assert_eq!(DataSize::calculate(i64::MIN), DataSize::I64);
        assert_eq!(DataSize::calculate(i64::MAX), DataSize::I64);
    }
}