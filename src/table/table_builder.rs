use crate::db::error::DbError;
use crate::table::block_builder::BlockBuilder;
use crate::table::filter_block::FilterBlockBuilder;
use crate::table::format::{BlockHandle, Footer};
use crate::util::options::CompressionType::NoCompression;
use crate::util::options::{CompressionType, Options};
use bytes::{Bytes, BytesMut};
use crc32fast::Hasher;
use std::cmp::Ordering;
use std::io::{Read, Write};
use zstd::zstd_safe::WriteBuf;

const K_BLOCK_TRAILER_SIZE: usize = 5;
const MASK_DELTA: u32 = 0xa282ead8;

/// Mask the CRC value to prevent issues with CRCs embedded in the data
fn mask(crc: u32) -> u32 {
    ((crc >> 15) | (crc << 17)).wrapping_add(MASK_DELTA)
}

/// Convert block contents and type into trailer bytes
fn create_block_trailer(block_contents: &[u8], block_type: u8) -> [u8; 5] {
    let mut trailer = [0u8; 5];
    trailer[0] = block_type;

    // Calculate initial CRC of block contents
    let mut hasher = Hasher::new();
    hasher.update(block_contents);

    // Extend CRC to cover block type
    hasher.update(&[block_type]);

    let crc = hasher.finalize();
    let masked_crc = mask(crc);

    // Encode masked CRC in little-endian
    trailer[1..].copy_from_slice(&masked_crc.to_le_bytes());

    trailer
}

pub struct TableBuilder<'a, T: Write> {
    options: Options,
    index_block_options: Options,
    file: &'a mut T,
    offset: u64,
    data_block: BlockBuilder,
    index_block: BlockBuilder,
    last_key: Vec<u8>,
    num_entries: i64,
    closed: bool,
    filter_block: Option<Box<FilterBlockBuilder>>,
    pending_index_entry: bool,
    pending_handle: BlockHandle,
    status: Result<(), DbError>,
}

impl<'a, T: Write> TableBuilder<'a, T> {
    pub fn new(options: Options, file: &'a mut T) -> Self {
        let mut index_block_options = options.clone();
        let data_block = BlockBuilder::new(options.clone());
        let index_block = BlockBuilder::new(index_block_options.clone());
        let pending_handle = BlockHandle::new();
        index_block_options.block_restart_interval = 1;
        let mut filter_block = None;
        if let Some(f) = &options.filter_policy {
            let mut fb = Box::new(FilterBlockBuilder::new(f.clone()));
            fb.start_block(0);
            filter_block = Some(fb);
        }
        TableBuilder {
            options,
            index_block_options,
            file,
            offset: 0,
            data_block,
            index_block,
            last_key: Vec::new(),
            num_entries: 0,
            closed: false,
            filter_block,
            pending_index_entry: false,
            pending_handle,
            status: Ok(()),
        }
    }

    pub fn change_options(&mut self, options: Options) -> Result<(), DbError> {
        // if options.comparator != self.options.comparator {
        //     return Err(DbError::InvalidArgument(
        //         "changing comparator while building table".to_string(),
        //     ));
        // }

        self.options = options.clone();
        self.index_block_options = options;
        self.index_block_options.block_restart_interval = 1;
        Ok(())
    }

    pub fn add(&mut self, key: &[u8], value: &[u8]) {
        assert!(!self.closed);
        if self.status.is_err() {
            return;
        }

        println!("table add key: {:?}", key);
        println!("table add value: {:?}", value);

        if self.num_entries > 0 {
            assert_eq!(
                self.options
                    .comparator
                    .compare(key, self.last_key.as_slice()),
                Ordering::Greater
            );
        }

        if self.pending_index_entry {
            assert!(self.data_block.empty());
            let sep = self
                .options
                .comparator
                .find_shortest_separator(self.last_key.as_slice(), key);
            if let Some(s) = sep {
                self.last_key = s;
            }
            let handle_ending = self.pending_handle.encode_to();
            self.index_block
                .add(self.last_key.as_slice(), handle_ending.as_slice());
            self.pending_index_entry = false;
        }

        if let Some(filter) = &mut self.filter_block {
            filter.add_key(key);
        }

        self.last_key = key.to_vec();
        self.num_entries += 1;
        self.data_block.add(key, value);

        let estimated_block_size = self.data_block.current_size_estimate();
        if estimated_block_size >= self.options.block_size as usize {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        assert!(!self.closed);
        if self.status.is_err() {
            return;
        }

        if self.data_block.empty() {
            return;
        }
        assert!(!self.pending_index_entry);
        println!("flush data block");
        let raw = self.data_block.finish();
        println!("flush data block raw: {:?}", raw);
        self.pending_handle = self.write_block(raw);
        self.data_block.reset();
        //self.pending_handle = self.write_data_block();
        if self.status.is_ok() {
            self.pending_index_entry = true;
            self.status = self.file.flush().map_err(DbError::IOError);
        }

        if let Some(filter) = &mut self.filter_block {
            filter.start_block(self.offset);
        }
    }

    pub fn write_block(&mut self, raw: Bytes) -> BlockHandle {
        // File format contains a sequence of blocks where each block has:
        //    block_data: uint8[n]
        //    type: uint8
        //    crc: uint32
        //let raw = block.finish();
        let mut t = self.options.compression.clone();
        let mut compressed_output = Vec::new();
        let use_compression = match t {
            CompressionType::NoCompression => false,
            CompressionType::SnappyCompression => {
                println!("write block snappy");
                let mut encoder = snap::write::FrameEncoder::new(&mut compressed_output);
                match encoder.write_all(raw.as_slice()) {
                    Ok(_) => {
                        if encoder.flush().is_err() {
                            t = NoCompression;
                            false
                        } else {
                            t = CompressionType::SnappyCompression;
                            true
                        }
                    }
                    Err(_) => {
                        t = NoCompression;
                        false
                    }
                }
            }
            CompressionType::ZstdCompression => {
                match zstd::stream::copy_encode(
                    raw.as_slice(),
                    &mut compressed_output,
                    self.options.zstd_compression_level,
                ) {
                    Ok(_) => {
                        t = CompressionType::ZstdCompression;
                        true
                    }
                    Err(_) => {
                        t = NoCompression;
                        false
                    }
                }
            }
        };

        let handle = if use_compression && compressed_output.len() < raw.len() - (raw.len() / 8) {
            println!("write block snappy {:?}", compressed_output);
            self.write_raw_block(compressed_output.as_slice(), t)
        } else {
            t = NoCompression;
            println!("write block snappy raw {:?}", t);
            self.write_raw_block(raw.as_slice(), t)
        };
        //block.reset();
        handle
    }

    pub fn write_raw_block(&mut self, block_contents: &[u8], ct: CompressionType) -> BlockHandle {
        let mut handle = BlockHandle::new();
        handle.set_offset(self.offset);
        handle.set_size(block_contents.len() as u64);
        self.status = self
            .file
            .write_all(block_contents)
            .map_err(DbError::IOError);
        if self.status.is_ok() {
            let trailer = create_block_trailer(block_contents, ct as u8);
            self.status = self.file.write_all(&trailer).map_err(DbError::IOError);
            if self.status.is_ok() {
                self.offset += block_contents.len() as u64 + K_BLOCK_TRAILER_SIZE as u64;
            }
        }
        handle
    }

    pub fn finish(&mut self) -> &Result<(), DbError> {
        self.flush();
        assert!(!self.closed);
        self.closed = true;

        let mut filter_block_handle = BlockHandle::new();
        if self.status.is_ok() {
            if let Some(filter) = &mut self.filter_block.take() {
                filter_block_handle =
                    self.write_raw_block(filter.finish(), CompressionType::NoCompression);
            }
        }

        let mut metaindex_block_handle = BlockHandle::new();
        if self.status.is_ok() {
            let mut meta_index_block = BlockBuilder::new(self.options.clone());
            if let Some(filter) = &self.filter_block {
                let mut filter_key = b"filter.".to_vec();
                if let Some(fp) = &self.options.filter_policy {
                    filter_key.extend_from_slice(fp.name().as_bytes());
                }
                let handle_encoding = filter_block_handle.encode_to();
                meta_index_block.add(filter_key.as_slice(), handle_encoding.as_slice());
            }

            let raw = meta_index_block.finish();
            metaindex_block_handle = self.write_block(raw);
            meta_index_block.reset();
        }

        let mut index_block_handle = BlockHandle::new();
        if self.status.is_ok() {
            if self.pending_index_entry {
                if let Some(s) = self
                    .options
                    .comparator
                    .find_short_successor(self.last_key.as_slice())
                {
                    self.last_key = s;
                }
                let handle_encoding = self.pending_handle.encode_to();
                println!("last_key: {:?}", self.last_key);
                self.index_block
                    .add(self.last_key.as_slice(), handle_encoding.as_slice());
                self.pending_index_entry = false;
            }

            let raw = self.index_block.finish();
            index_block_handle = self.write_block(raw);
            self.index_block.reset();
        }

        if self.status.is_ok() {
            let mut footer = Footer::new();
            footer.set_index_handle(index_block_handle);
            footer.set_metaindex_handle(metaindex_block_handle);
            let footer_encoding = footer.encode_to();
            println!("footer_encoding: {:?}", footer_encoding);
            println!("footer_encoding footer: {:?}", footer);
            self.status = self
                .file
                .write_all(footer_encoding.as_slice())
                .map_err(DbError::IOError);
            if self.status.is_ok() {
                self.offset += footer_encoding.len() as u64;
            }
        }

        &self.status
    }

    pub fn status(&self) -> &Result<(), DbError> {
        &self.status
    }

    pub fn file_size(&self) -> u64 {
        self.offset
    }
}
