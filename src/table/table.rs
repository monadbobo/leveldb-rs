use crate::db::error::DbError;
use crate::db::iterator::DBIterator;
use crate::db::options::ReadOptions;
use crate::table::block::Block;
use crate::table::filter_block::FilterBlockReader;
use crate::table::format::{kEncodedLength, read_block, BlockHandle, Footer};
use crate::table::two_level_iterator::BlockFunction;
use crate::util::coding::encode_fixed64;
use crate::util::comparator::{BytewiseComparatorImpl, Comparator};
use crate::util::options::Options;
use std::os::unix::fs::FileExt;
use std::sync::Arc;

pub trait HanldeTableData {
    fn handle(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError>;
}

pub struct Table<'a> {
    options: Options,
    status: Result<(), DbError>,
    file: std::fs::File,
    cache_id: u64,
    filter: Option<FilterBlockReader<'a>>,
    filter_data: Option<Vec<u8>>,
    metaindex_handle: BlockHandle,
    index_block: Block,
}

impl<'a> Table<'a> {
    pub fn new(options: &Options, file: std::fs::File, size: u64) -> Result<Table, DbError> {
        if size < kEncodedLength as u64 {
            return Err(DbError::Corruption(
                "file is too short to be an sstable".to_string(),
            ));
        }

        let mut buf = [0u8; kEncodedLength];
        file.read_at(&mut buf, size - kEncodedLength as u64)?;
        let mut footer = Footer::new();
        footer.decode_from(&buf)?;

        let mut opt = ReadOptions::default();
        if options.paranoid_checks {
            opt.verify_checksums = true;
        }
        let index_block_contents = read_block(&file, &opt, &footer.index_handle)?;

        let index_block = Block::new(index_block_contents);
        let cache_id = if let Some(cache) = &options.block_cache {
            cache.new_id()
        } else {
            0
        };
        let table = Table {
            options: options.clone(),
            status: Ok(()),
            file,
            cache_id,
            filter: None,
            filter_data: None,
            metaindex_handle: footer.metaindex_handle,
            index_block,
        };
        Ok(table)
    }

    pub fn read_meta(&'a mut self, footer: &Footer) {
        if self.options.filter_policy.is_none() {
            return;
        }

        let mut opt = ReadOptions::default();
        if self.options.paranoid_checks {
            opt.verify_checksums = true;
        }

        if let Ok(contents) = read_block(&self.file, &opt, &footer.metaindex_handle) {
            let meta = Block::new(contents);
            let bytes_comparator: Arc<Box<dyn Comparator>> =
                Arc::new(Box::new(BytewiseComparatorImpl));
            let mut iter = meta.new_iterator(bytes_comparator);
            let mut key = b"filter.".to_vec();
            key.extend_from_slice(
                self.options
                    .filter_policy
                    .as_ref()
                    .unwrap()
                    .name()
                    .as_bytes(),
            );

            iter.seek(&key);
            if iter.valid() && iter.key() == key {
                self.read_filter(iter.value());
            }
        }
    }

    pub fn read_filter(&'a mut self, filter_handle_value: &[u8]) {
        let mut filter_handle = BlockHandle::new();
        if filter_handle.decode_from(filter_handle_value).is_err() {
            return;
        }

        let mut opt = ReadOptions::default();
        if self.options.paranoid_checks {
            opt.verify_checksums = true;
        }

        if let Ok(block) = read_block(&self.file, &opt, &filter_handle) {
            self.filter_data = Some(block.data);
            self.filter = Some(FilterBlockReader::new(
                self.options.filter_policy.as_ref().unwrap().clone(),
                self.filter_data.as_ref().unwrap(),
            ));
        }
    }

    pub fn internal_get(
        &mut self,
        options: &ReadOptions,
        k: &[u8],
        mut handle: Box<dyn HanldeTableData>,
    ) -> Result<(), DbError> {
        let mut iiter = self
            .index_block
            .new_iterator(self.options.comparator.clone());
        iiter.seek(k);
        if iiter.valid() {
            let handle_value = iiter.value();
            let mut not_found = false;
            if let Some(filter) = self.filter.as_ref() {
                let mut handle = BlockHandle::new();
                if handle.decode_from(handle_value).is_ok()
                    && filter.key_may_match(handle.offset, k)
                {
                    not_found = true;
                }
            }

            if !not_found {
                let mut block_iter = self.new_iterator(options, iiter.value())?;
                block_iter.seek(k);
                if block_iter.valid() {
                    handle.handle(block_iter.key(), block_iter.value())?;
                }
            }
        }

        iiter.status()
    }

    pub fn approximate_offset_of(&self, key: &[u8]) -> u64 {
        let mut index_iter = self
            .index_block
            .new_iterator(self.options.comparator.clone());
        index_iter.seek(key);

        if index_iter.valid() {
            let input = index_iter.value();
            let mut handle = BlockHandle::new();
            if handle.decode_from(input).is_ok() {
                handle.offset
            } else {
                self.metaindex_handle.offset
            }
        } else {
            self.metaindex_handle.offset
        }
    }
}

impl BlockFunction for Table<'_> {
    fn new_iterator(
        &self,
        options: &ReadOptions,
        index_value: &[u8],
    ) -> Result<Box<dyn DBIterator>, DbError> {
        let block_cache = self.options.block_cache.as_ref();

        let mut handle = BlockHandle::new();
        handle.decode_from(index_value)?;
        let block = if let Some(cache) = block_cache {
            let mut cache_key_buffer = Vec::with_capacity(16);
            cache_key_buffer.append(&mut encode_fixed64(self.cache_id));
            cache_key_buffer.append(&mut encode_fixed64(handle.offset));
            if let Some(cache_handle) = cache.lookup(cache_key_buffer.as_slice()) {
                todo!()
            } else {
                let contents = read_block(&self.file, &options, &handle)?;

                Block::new(contents)
            }
        } else {
            let contents = read_block(&self.file, &options, &handle)?;
            Block::new(contents)
        };

        Ok(block.new_iterator_into(Arc::new(Box::new(BytewiseComparatorImpl))))
    }
}
