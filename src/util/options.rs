#[derive(Debug, Clone)]
pub struct Options {
    pub(crate) max_file_size: isize,
    pub(crate) block_restart_interval: isize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_file_size: 4 * 1024 * 1024,
            block_restart_interval: 16,
        }
    }
}

#[derive(Debug, Clone)]
#[repr(u8)]
pub enum CompressionType {
    NoCompression = 0,
    SnappyCompression = 1,
    ZstdCompression = 2,
}

impl TryFrom<u8> for CompressionType {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(CompressionType::NoCompression),
            1 => Ok(CompressionType::SnappyCompression),
            2 => Ok(CompressionType::ZstdCompression),
            _ => Err("bad block type"),
        }
    }
}
