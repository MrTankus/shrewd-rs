
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    InvalidDataSize(u8),
    InvalidCompressorType(u8),
}