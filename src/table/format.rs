use crate::db::error::DbError;
use crate::db::error::DbError::Corruption;
use crate::db::options::ReadOptions;
use crate::util::coding::{decode_fixed32, get_varint64, put_fixed32, put_varint64};
use crate::util::options::CompressionType;
use std::convert::Infallible;
use std::f32::consts::E;
use std::io::Read;
use std::os::unix::fs::FileExt;

pub struct BlockContent {
    pub(crate) data: Vec<u8>,
    cachable: bool,
    pub(crate) heap_allocated: bool,
}

// BlockHandle is a pointer to the extent of a file that stores a data
// block or a meta block.
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

    pub fn decode_from(&mut self, data: &[u8]) -> Result<(), DbError> {
        match get_varint64(data) {
            None => Err(Corruption("bad block handle".to_string())),
            Some((s, offset)) => match get_varint64(&data[s..]) {
                None => Err(Corruption("bad block handle".to_string())),
                Some((s, size)) => {
                    self.offset = offset;
                    self.size = size;
                    Ok(())
                }
            },
        }
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

const kEncodedLength: usize = 2 * kMaxEncodedLength + 8;

const kTableMagicNumber: u64 = 0xdb4775248b80fb57;

// 1-byte type + 32-bit crc
const kBlockTrailerSize: usize = 5;

// Footer encapsulates the fixed information stored at the tail
// end of every table file.
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
        let magic_slice = &data[data.len() + kEncodedLength - 8..];
        let magic_lo = decode_fixed32(&magic_slice[..4]);
        let magic_hi = decode_fixed32(&magic_slice[4..]);
        let magic = (magic_lo as u64) | ((magic_hi as u64) << 32);
        if magic != kTableMagicNumber {
            return Err(Corruption("not an sstable (bad magic number)".to_string()));
        }

        self.metaindex_handle
            .decode_from(&data[..kMaxEncodedLength])?;

        self.index_handle.decode_from(&data[kMaxEncodedLength..])?;

        // We skip over any leftover data (just padding for now) in "data" todo!()
        Ok(())
    }
}

pub fn read_block(
    file: &std::fs::File,
    options: &ReadOptions,
    handle: &BlockHandle,
) -> Result<BlockContent, DbError> {
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
            let data = buffer[..handle.size as usize].to_vec();
            Ok(BlockContent {
                data,
                cachable: true,
                heap_allocated: true,
            })
        }
        Ok(CompressionType::SnappyCompression) => {
            let mut r = snap::read::FrameDecoder::new(&buffer[..handle.size as usize]);
            let mut data = Vec::new();
            r.read(&mut data)?;
            Ok(BlockContent {
                data,
                cachable: true,
                heap_allocated: true,
            })
        }
        Ok(CompressionType::ZstdCompression) => {
            let mut r = zstd::stream::Decoder::new(&buffer[..handle.size as usize])?;
            let mut data = Vec::new();
            r.read_to_end(&mut data)?;
            Ok(BlockContent {
                data,
                cachable: true,
                heap_allocated: true,
            })
        }
        Err(_) => Err(Corruption("bad block type".to_string())),
    }
}
