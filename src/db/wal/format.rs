use std::fmt::{Display, Formatter};
use once_cell::sync::Lazy;


pub static MAX_RECORD_TYPE: Lazy<u8> = Lazy::new(||RecordType::Last as u8);

pub const BLOCK_SIZE: u64 = 32768;
pub const HEADER_SIZE: usize = 4 + 2 + 1;

#[derive(PartialEq, Debug)]
#[repr(u8)]
pub enum RecordType {
    Zero = 0,
    Full,
    First,
    Middle,
    Last,
}

impl Display for RecordType {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            RecordType::Zero => f.write_str("zero"),
            RecordType::Full => f.write_str("full"),
            RecordType::First => f.write_str("first"),
            RecordType::Middle => f.write_str("middle"),
            RecordType::Last => f.write_str("last"),
        }
    }
}

impl TryFrom<u8> for RecordType {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(RecordType::Zero),
            1 => Ok(RecordType::Full),
            2 => Ok(RecordType::First),
            3 => Ok(RecordType::Middle),
            4 => Ok(RecordType::Last),
            _ => Err("Invalid record type"),
        }
    }
}
