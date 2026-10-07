use super::errors::DecodeError;

/// Cursor over an encoded buffer. Every read consumes bytes from the front and fails with
/// `DecodeError::UnexpectedEOF` instead of panicking when the buffer is too short.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub(crate) fn read_array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (head, rest) = self.data.split_first_chunk::<N>().ok_or(DecodeError::UnexpectedEOF)?;
        self.data = rest;
        Ok(*head)
    }

    #[inline]
    pub(crate) fn read_u8(&mut self) -> Result<u8, DecodeError> {
        let [byte] = self.read_array()?;
        Ok(byte)
    }

    /// Reads a length stored as a little-endian `u64`. A length that does not fit in `usize`
    /// cannot describe data that is in memory, so it is reported as the input ending early.
    #[inline]
    pub(crate) fn read_len(&mut self) -> Result<usize, DecodeError> {
        let len = u64::from_le_bytes(self.read_array()?);
        usize::try_from(len).map_err(|_| DecodeError::UnexpectedEOF)
    }

    #[inline]
    pub(crate) fn read_i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.read_array()?))
    }

    pub(crate) fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let (head, rest) = self.data.split_at_checked(len).ok_or(DecodeError::UnexpectedEOF)?;
        self.data = rest;
        Ok(head)
    }

    /// Reads `count` elements of `width` bytes each, guarding against a corrupted `count`
    /// overflowing the byte length.
    pub(crate) fn read_packed(&mut self, count: usize, width: usize) -> Result<&'a [u8], DecodeError> {
        let len = count.checked_mul(width).ok_or(DecodeError::UnexpectedEOF)?;
        self.read_bytes(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reads_advance_through_buffer() {
        let mut bytes = vec![7u8];
        bytes.extend_from_slice(&42u64.to_le_bytes());
        bytes.extend_from_slice(&(-5i64).to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4]);

        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.read_u8(), Ok(7));
        assert_eq!(reader.read_len(), Ok(42));
        assert_eq!(reader.read_i64(), Ok(-5));
        assert_eq!(reader.read_packed(2, 2), Ok(&[1u8, 2, 3, 4][..]));
        assert_eq!(reader.read_u8(), Err(DecodeError::UnexpectedEOF));
    }

    #[test]
    fn test_short_buffer_is_eof() {
        assert_eq!(Reader::new(&[]).read_u8(), Err(DecodeError::UnexpectedEOF));
        assert_eq!(Reader::new(&[0; 7]).read_i64(), Err(DecodeError::UnexpectedEOF));
        assert_eq!(Reader::new(&[0; 3]).read_bytes(4), Err(DecodeError::UnexpectedEOF));
    }

    #[test]
    fn test_packed_length_overflow_is_eof() {
        assert_eq!(Reader::new(&[0; 8]).read_packed(usize::MAX, 2), Err(DecodeError::UnexpectedEOF));
    }
}
