use crate::util::cache::Cache;
use crate::util::comparator::{BytewiseComparatorImpl, Comparator};
use crate::util::filter_policy::FilterPolicy;
use crate::util::options::CompressionType::SnappyCompression;
use std::sync::Arc;

#[derive(Clone)]
pub struct Options {
    pub max_file_size: isize,
    pub block_restart_interval: isize,
    pub filter_policy: Option<Arc<Box<dyn FilterPolicy>>>,
    pub comparator: Arc<Box<dyn Comparator>>,
    pub block_size: isize,
    pub compression: CompressionType,
    pub zstd_compression_level: i32,
    pub paranoid_checks: bool,
    pub block_cache: Option<Arc<dyn Cache<Vec<u8>, Vec<u8>>>>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_file_size: 4 * 1024 * 1024,
            block_restart_interval: 16,
            filter_policy: None,
            comparator: Arc::new(Box::new(BytewiseComparatorImpl)),
            block_size: 4 * 1024,
            compression: SnappyCompression,
            zstd_compression_level: 1,
            paranoid_checks: false,
            block_cache: None,
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
