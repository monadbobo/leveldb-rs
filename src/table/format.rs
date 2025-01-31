use crate::db::error::DbError;
use crate::db::error::DbError::Corruption;
use crate::db::options::ReadOptions;
use crate::util::coding::{decode_fixed32, get_varint64, put_fixed32, put_varint64};
use crate::util::options::CompressionType;
use std::convert::Infallible;
use std::f32::consts::E;
use std::io::{Read, Seek};
use std::os::unix::fs::FileExt;

pub struct BlockContent {
    pub(crate) data: Vec<u8>,
    cachable: bool,
    pub(crate) heap_allocated: bool,
}

impl BlockContent {
    pub fn new(data: Vec<u8>, cachable: bool, heap_allocated: bool) -> Self {
        BlockContent {
            data,
            cachable,
            heap_allocated,
        }
    }
}

// BlockHandle is a pointer to the extent of a file that stores a data
// block or a meta block.
#[derive(Debug, Clone)]
pub struct BlockHandle {
    pub(crate) offset: u64,
    pub(crate) size: u64,
}

const kMaxEncodedLength: usize = 10 + 10;

impl BlockHandle {
    pub fn new() -> Self {
        BlockHandle { offset: 0, size: 0 }
    }

    pub fn encode_to(&self) -> Vec<u8> {
        let mut result = put_varint64(self.offset);
        result.append(&mut put_varint64(self.size));
        result
    }

    pub fn decode_from(&mut self, data: &[u8]) -> Result<usize, DbError> {
        // 解码 offset
        let (s1, offset) = get_varint64(data)
            .ok_or_else(|| Corruption("bad block handle (offset)".to_string()))?;

        // 解码 size（注意剩余数据长度）
        let remaining = data
            .get(s1..)
            .ok_or_else(|| Corruption("bad block handle (insufficient data)".to_string()))?;

        let (s2, size) = get_varint64(remaining)
            .ok_or_else(|| Corruption("bad block handle (size)".to_string()))?;

        // 更新字段并返回总消耗字节数
        self.offset = offset;
        self.size = size;
        Ok(s1 + s2)
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn set_offset(&mut self, offset: u64) {
        self.offset = offset;
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn set_size(&mut self, size: u64) {
        self.size = size;
    }
}

// Encoded length of a Footer.  Note that the serialization of a
// Footer will always occupy exactly this many bytes.  It consists
// of two block handles and a magic number.

pub(crate) const kEncodedLength: usize = 2 * kMaxEncodedLength + 8;

pub(crate) const kTableMagicNumber: u64 = 0xdb4775248b80fb57;

// 1-byte type + 32-bit crc
pub(crate) const kBlockTrailerSize: usize = 5;

// Footer encapsulates the fixed information stored at the tail
// end of every table file.
#[derive(Debug, Clone)]
pub struct Footer {
    pub(crate) metaindex_handle: BlockHandle,
    pub(crate) index_handle: BlockHandle,
}

impl Footer {
    pub fn new() -> Self {
        Footer {
            metaindex_handle: BlockHandle::new(),
            index_handle: BlockHandle::new(),
        }
    }

    pub fn metaindex_handle(&self) -> &BlockHandle {
        &self.metaindex_handle
    }

    pub fn set_metaindex_handle(&mut self, handle: BlockHandle) {
        self.metaindex_handle = handle;
    }

    pub fn index_handle(&self) -> &BlockHandle {
        &self.index_handle
    }

    pub fn set_index_handle(&mut self, handle: BlockHandle) {
        self.index_handle = handle;
    }

    pub fn encode_to(&self) -> Vec<u8> {
        let mut result = self.metaindex_handle.encode_to();
        result.append(&mut self.index_handle.encode_to());
        result.resize(kMaxEncodedLength * 2, 0);
        result.append(&mut put_fixed32((kTableMagicNumber & 0xffffffff) as u32));
        result.append(&mut put_fixed32((kTableMagicNumber >> 32) as u32));
        result
    }

    pub fn decode_from(&mut self, data: &[u8]) -> Result<(), DbError> {
        if data.len() < kEncodedLength {
            return Err(Corruption("not an sstable (footer too short)".to_string()));
        }
        let magic_slice = &data[kEncodedLength - 8..];
        let magic_lo = decode_fixed32(&magic_slice[..4]);
        let magic_hi = decode_fixed32(&magic_slice[4..]);
        let magic = (magic_lo as u64) | ((magic_hi as u64) << 32);
        if magic != kTableMagicNumber {
            return Err(Corruption("not an sstable (bad magic number)".to_string()));
        }

        let consumed = self.metaindex_handle.decode_from(&data)?;

        self.index_handle.decode_from(&data[consumed..])?;

        // We skip over any leftover data (just padding for now) in "data" todo!()
        Ok(())
    }
}

pub fn read_block<T: FileExt>(
    file: &T,
    options: &ReadOptions,
    handle: &BlockHandle,
) -> Result<BlockContent, DbError> {
    println!("read_block: {:?}", handle);
    // Read the block contents as well as the type/crc footer.
    // See table_builder.cc for the code that built this structure.
    let mut buffer = vec![0; handle.size as usize + kBlockTrailerSize];

    let size = file.read_at(&mut buffer, handle.offset)?;
    if size != (handle.size + kBlockTrailerSize as u64) as usize {
        return Err(Corruption("truncated block read".to_string()));
    }

    if options.verify_checksums {
        todo!("verify checksums");
    }

    match CompressionType::try_from(buffer[handle.size as usize]) {
        Ok(CompressionType::NoCompression) => {
            println!("read_block: NoCompression");
            let data = buffer[..handle.size as usize].to_vec();
            Ok(BlockContent {
                data,
                cachable: true,
                heap_allocated: true,
            })
        }
        Ok(CompressionType::SnappyCompression) => {
            println!("read_block: SnappyCompression");
            let mut r = snap::read::FrameDecoder::new(&buffer[..handle.size as usize]);
            let mut data = Vec::new();
            r.read_to_end(&mut data)?;
            Ok(BlockContent {
                data,
                cachable: true,
                heap_allocated: true,
            })
        }
        Ok(CompressionType::ZstdCompression) => {
            println!("read_block: ZstdCompression");
            let mut r = zstd::stream::Decoder::new(&buffer[..handle.size as usize])?;
            let mut data = Vec::new();
            r.read_to_end(&mut data)?;
            println!("read_block snappy: {:?}", data);
            Ok(BlockContent {
                data,
                cachable: true,
                heap_allocated: true,
            })
        }
        Err(_) => Err(Corruption("bad block type".to_string())),
    }
}
