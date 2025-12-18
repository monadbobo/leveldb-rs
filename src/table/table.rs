use crate::db::error::DbError;
use crate::db::iterator::DBIterator;
use crate::db::options::ReadOptions;
use crate::table::block::Block;
use crate::table::filter_block::FilterBlockReader;
use crate::table::format::{kEncodedLength, read_block, BlockHandle, Footer};
use crate::table::two_level_iterator::{new_two_level_iterator, BlockFunction};
use crate::util::coding::encode_fixed64;
use crate::util::comparator::{BytewiseComparatorImpl, Comparator};
use crate::util::options::Options;
use std::io::{Read, Seek};
use std::os::unix::fs::FileExt;
use std::sync::Arc;

pub trait HanldeTableData {
    fn handle(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError>;
}

struct Rep<'a, T: FileExt> {
    options: Options,
    cache_id: u64,
    filter: Option<FilterBlockReader<'a>>,
    filter_data: Option<Vec<u8>>,
    metaindex_handle: BlockHandle,
    index_block: Block,
    file: T,
}

pub struct Table<'a, T: FileExt> {
    rep: Rep<'a, T>,
}

impl<'a, T: FileExt> Table<'a, T> {
    pub fn new(options: Options, mut file: T, size: u64) -> Result<Table<'a, T>, DbError> {
        if size < kEncodedLength as u64 {
            return Err(DbError::Corruption(
                "file is too short to be an sstable".to_string(),
            ));
        }

        let mut buf = [0u8; kEncodedLength];
        file.read_at(&mut buf, size - kEncodedLength as u64)?;
        println!("Footer Buf: {:?}", buf);
        let mut footer = Footer::new();
        footer.decode_from(&buf)?;
        println!("Footer: {:?}", footer);

        let mut opt = ReadOptions::default();
        if options.paranoid_checks {
            opt.verify_checksums = true;
        }
        let index_block_contents = read_block(&mut file, &opt, &footer.index_handle)?;
        println!(
            "Index block contents: {:?}",
            index_block_contents.data.len()
        );

        let index_block = Block::new(index_block_contents);
        let cache_id = if let Some(cache) = &options.block_cache {
            cache.new_id()
        } else {
            0
        };

        println!("index_block {:?}", index_block);
        let rep = Rep {
            options,
            cache_id,
            filter: None,
            filter_data: None,
            metaindex_handle: footer.metaindex_handle,
            index_block,
            file,
        };
        let table = Table { rep };
        Ok(table)
    }

    pub fn new_iterator<'b>(&'b self, options: ReadOptions) -> Box<dyn DBIterator + 'b> {
        let iter = self
            .rep
            .index_block
            .new_iterator(self.rep.options.comparator.clone());
        new_two_level_iterator(iter, self, options)
    }

    pub fn read_meta(&'a mut self, footer: &Footer) {
        if self.rep.options.filter_policy.is_none() {
            return;
        }

        let mut opt = ReadOptions::default();
        if self.rep.options.paranoid_checks {
            opt.verify_checksums = true;
        }

        if let Ok(contents) = read_block(&self.rep.file, &opt, &footer.metaindex_handle) {
            let meta = Block::new(contents);
            let bytes_comparator: Arc<Box<dyn Comparator>> =
                Arc::new(Box::new(BytewiseComparatorImpl));
            let mut iter = meta.new_iterator(bytes_comparator);
            let mut key = b"filter.".to_vec();
            key.extend_from_slice(
                self.rep
                    .options
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
        if self.rep.options.paranoid_checks {
            opt.verify_checksums = true;
        }

        if let Ok(block) = read_block(&self.rep.file, &opt, &filter_handle) {
            self.rep.filter_data = Some(block.data);
            self.rep.filter = Some(FilterBlockReader::new(
                self.rep.options.filter_policy.as_ref().unwrap().clone(),
                self.rep.filter_data.as_ref().unwrap(),
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
            .rep
            .index_block
            .new_iterator(self.rep.options.comparator.clone());
        iiter.seek(k);
        if iiter.valid() {
            let handle_value = iiter.value();
            let mut not_found = false;
            if let Some(filter) = self.rep.filter.as_ref() {
                let mut handle = BlockHandle::new();
                if handle.decode_from(handle_value).is_ok()
                    && filter.key_may_match(handle.offset, k)
                {
                    not_found = true;
                }
            }

            if !not_found {
                let mut block_iter = self.block_iterator(options, iiter.value())?;
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
            .rep
            .index_block
            .new_iterator(self.rep.options.comparator.clone());
        index_iter.seek(key);

        if index_iter.valid() {
            let input = index_iter.value();
            let mut handle = BlockHandle::new();
            if handle.decode_from(input).is_ok() {
                handle.offset
            } else {
                self.rep.metaindex_handle.offset
            }
        } else {
            self.rep.metaindex_handle.offset
        }
    }
}

impl<T: FileExt> BlockFunction for Table<'_, T> {
    fn block_iterator(
        &self,
        options: &ReadOptions,
        index_value: &[u8],
    ) -> Result<Box<dyn DBIterator>, DbError> {
        let block_cache = self.rep.options.block_cache.as_ref();

        let mut handle = BlockHandle::new();
        handle.decode_from(index_value)?;
        let block = if let Some(cache) = block_cache {
            let mut cache_key_buffer = Vec::with_capacity(16);
            cache_key_buffer.append(&mut encode_fixed64(self.rep.cache_id));
            cache_key_buffer.append(&mut encode_fixed64(handle.offset));
            if let Some(cache_handle) = cache.lookup(&cache_key_buffer) {
                todo!()
            } else {
                let contents = read_block(&self.rep.file, &options, &handle)?;

                Block::new(contents)
            }
        } else {
            let contents = read_block(&self.rep.file, &options, &handle)?;
            Block::new(contents)
        };

        Ok(block.new_iterator_into(Arc::new(Box::new(BytewiseComparatorImpl))))
    }
}

#[cfg(test)]
mod tests {
    use crate::db::dbformat::ValueType::TypeValue;
    use crate::db::dbformat::{
        append_internal_key, parse_internal_key, InternalKeyComparator, ParseInternalKey,
        MAX_SEQUENCE_NUMBER,
    };
    use crate::db::error::DbError;
    use crate::db::iterator::DBIterator;
    use crate::db::memtable::MemTable;
    use crate::db::options::ReadOptions;
    use crate::table::block::Block;
    use crate::table::block_builder::BlockBuilder;
    use crate::table::format::BlockContent;
    use crate::table::table::Table;
    use crate::table::table_builder::TableBuilder;
    use crate::util::comparator::{BytewiseComparatorImpl, Comparator};
    use crate::util::options::{CompressionType, Options};
    use crate::util::testutil::{compressible_string, random_key, random_string, skewed};
    use bytes::{BufMut, Bytes};
    use rand::thread_rng;
    use std::any::Any;
    use std::cmp::Ordering;
    use std::io::Write;
    use std::os::unix::fs::FileExt;
    use std::rc::Rc;
    use std::sync::Arc;
    use zstd::zstd_safe::WriteBuf;

    struct ReverseKeyComparator;

    impl Comparator for ReverseKeyComparator {
        fn name(&self) -> &str {
            "leveldb.ReverseBytewiseCOmparator"
        }

        fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
            let mut a = a.to_vec();
            a.reverse();
            let mut b = b.to_vec();
            b.reverse();
            BytewiseComparatorImpl.compare(a.as_slice(), b.as_slice())
        }

        fn find_shortest_separator(&self, start: &[u8], limit: &[u8]) -> Option<Vec<u8>> {
            let mut s = start.to_vec();
            s.reverse();
            let mut l = limit.to_vec();
            l.reverse();
            BytewiseComparatorImpl
                .find_shortest_separator(s.as_slice(), l.as_slice())
                .map(|v| {
                    let mut v = v.clone();
                    v.reverse();
                    v
                })
        }

        fn find_short_successor(&self, key: &[u8]) -> Option<Vec<u8>> {
            let mut s = key.to_vec();
            s.reverse();
            BytewiseComparatorImpl
                .find_short_successor(s.as_slice())
                .map(|v| {
                    let mut v = v.clone();
                    v.reverse();
                    v
                })
        }
    }

    fn increment(cmp: Box<dyn Comparator>, key: &[u8]) -> Vec<u8> {
        if cmp.type_id() == BytewiseComparatorImpl.type_id() {
            let mut result = key.to_vec();
            result.push(b'\0');
            result
        } else {
            let mut rev = key.to_vec();
            rev.reverse();
            rev.put_u8(b'\0');
            rev
        }
    }

    #[derive(Clone)]
    struct StringSource {
        content: Vec<u8>,
    }

    impl FileExt for StringSource {
        fn read_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
            let mut len = buf.len();
            if offset + len as u64 > self.content.len() as u64 {
                len = (self.content.len() as u64 - offset) as usize;
            }
            buf.copy_from_slice(&self.content[offset as usize..offset as usize + len]);
            Ok(len)
        }

        fn write_at(&self, buf: &[u8], offset: u64) -> std::io::Result<usize> {
            todo!()
        }
    }

    struct StringSink {
        content: Vec<u8>,
    }

    impl Write for StringSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.content.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct KeyWrapper {
        key: Vec<u8>,
        cmp: Arc<Box<dyn Comparator>>,
    }

    impl KeyWrapper {
        pub fn new() -> Self {
            KeyWrapper {
                key: Vec::new(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            }
        }
    }

    impl PartialEq for KeyWrapper {
        fn eq(&self, other: &Self) -> bool {
            self.cmp.compare(self.key.as_slice(), other.key.as_slice()) == Ordering::Equal
        }
    }

    impl Eq for KeyWrapper {}

    impl PartialOrd for KeyWrapper {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for KeyWrapper {
        fn cmp(&self, other: &Self) -> Ordering {
            self.cmp.compare(self.key.as_slice(), other.key.as_slice())
        }
    }

    type KVMap = std::collections::BTreeMap<KeyWrapper, Vec<u8>>;

    pub trait Constructor {
        fn add(&mut self, key: KeyWrapper, value: &[u8]);
        fn finish_impl(&mut self, options: &Options, data: &KVMap) -> Result<(), DbError>;
        fn new_iterator(&self) -> Box<dyn DBIterator + '_>;
        fn data(&self) -> &KVMap;
        // fn db(&self) -> Option<&DB> {
        //     None
        // }

        fn finish(
            &mut self,
            options: &Options,
            keys: &mut Vec<KeyWrapper>,
            kvmap: &mut KVMap,
        ) -> Result<(), DbError> {
            *kvmap = self.data().clone();
            keys.clear();
            keys.extend(self.data().keys().cloned());
            self.finish_impl(options, kvmap)
        }
    }

    pub struct BlockConstructor {
        data: KVMap,
        comparator: Arc<Box<dyn Comparator>>,
        block: Option<Block>,
        block_data: Bytes,
    }

    impl BlockConstructor {
        pub fn new(cmp: Arc<Box<dyn Comparator>>) -> Self {
            BlockConstructor {
                data: KVMap::new(),
                comparator: cmp,
                block: None,
                block_data: Bytes::new(),
            }
        }
    }

    impl Constructor for BlockConstructor {
        fn add(&mut self, key: KeyWrapper, value: &[u8]) {
            self.data.insert(key, value.to_vec());
        }

        fn finish_impl(&mut self, options: &Options, data: &KVMap) -> Result<(), DbError> {
            self.data.clear();
            self.block = None;
            let mut builder = BlockBuilder::new(options.clone());

            for (key, value) in data {
                println!("{:?} value: {:?}", key.key.as_slice(), value);
                builder.add(key.key.as_slice(), value);
            }

            self.block_data = builder.finish();
            println!("Block data: {:?}", self.block_data.len());
            let content = BlockContent::new(self.block_data.to_vec(), false, false);

            self.block = Some(Block::new(content));
            println!("Block: {:?}", self.block);
            Ok(())
        }

        fn new_iterator(&self) -> Box<dyn DBIterator + '_> {
            self.block
                .as_ref()
                .unwrap()
                .new_iterator(self.comparator.clone())
        }

        fn data(&self) -> &KVMap {
            &self.data
        }
    }

    struct TableConstructor {
        comparator: Arc<Box<dyn Comparator>>,
        table: Option<Table<'static, StringSource>>,
        source: StringSource,
        data: KVMap,
    }

    impl TableConstructor {
        pub fn new(cmp: Arc<Box<dyn Comparator>>) -> Self {
            TableConstructor {
                comparator: cmp,
                table: None,
                data: KVMap::new(),
                source: StringSource {
                    content: Vec::new(),
                },
            }
        }

        pub fn approximate_offset_of_plain(&self, key: &[u8]) -> u64 {
            self.table.as_ref().unwrap().approximate_offset_of(key)
        }
    }

    impl Constructor for TableConstructor {
        fn add(&mut self, key: KeyWrapper, value: &[u8]) {
            println!("Add key: {:?}", key.key);
            println!("Add value: {:?}", value.to_vec());
            self.data.insert(key, value.to_vec());
        }

        fn finish_impl(&mut self, options: &Options, data: &KVMap) -> Result<(), DbError> {
            self.table = None;
            self.source.content.clear();

            let mut sink = StringSink {
                content: Vec::new(),
            };

            let file_size = {
                let mut table_builder = TableBuilder::new(options.clone(), &mut sink);

                for (key, value) in data {
                    table_builder.add(key.key.as_slice(), value.as_slice());
                    assert!(table_builder.status().is_ok());
                }

                assert!(table_builder.finish().is_ok());
                table_builder.file_size()
            };

            let len = sink.content.len();
            assert_eq!(len, file_size as usize);

            self.source = StringSource {
                content: sink.content,
            };

            let table_options = Options {
                comparator: self.comparator.clone(),
                ..options.clone()
            };

            Table::new(table_options, self.source.clone(), len as u64).map(|table| {
                self.table = Some(table);
            })
        }

        fn new_iterator(&self) -> Box<dyn DBIterator + '_> {
            self.table
                .as_ref()
                .unwrap()
                .new_iterator(ReadOptions::default())
        }

        fn data(&self) -> &KVMap {
            &self.data
        }
    }

    struct KeyConvertingIterator {
        iter: Box<dyn DBIterator>,
        status: Result<(), DbError>,
    }

    impl DBIterator for KeyConvertingIterator {
        fn valid(&self) -> bool {
            self.iter.valid()
        }

        fn seek_to_first(&mut self) {
            self.iter.seek_to_first()
        }

        fn seek_to_last(&mut self) {
            self.iter.seek_to_last()
        }

        fn seek(&mut self, target: &[u8]) {
            let ikey = ParseInternalKey::new(target, MAX_SEQUENCE_NUMBER, TypeValue);
            let key = append_internal_key(&ikey);
            self.iter.seek(key.as_slice())
        }

        fn next(&mut self) {
            self.iter.next()
        }

        fn prev(&mut self) {
            self.iter.prev()
        }

        fn key(&self) -> &[u8] {
            assert!(self.valid());
            match parse_internal_key(self.iter.key()) {
                None => {
                    //unsafe {self.status = Err(DbError::Corruption("badkey".to_string()));
                    &[]
                }
                Some(k) => k.user_key.as_slice(),
            }
        }

        fn value(&self) -> &[u8] {
            self.iter.value()
        }

        fn status(&self) -> Result<(), DbError> {
            //self.status.clone()
            todo!()
        }
    }

    struct MemTableConstructor {
        internal_comparator: InternalKeyComparator,
        memtable: MemTable,
        data: KVMap,
    }

    impl MemTableConstructor {
        pub fn new(cmp: Rc<dyn Comparator>) -> Self {
            let internal_comparator = InternalKeyComparator {
                user_comparator: cmp.clone(),
            };
            let memtable = MemTable::new(&internal_comparator);
            MemTableConstructor {
                internal_comparator,
                memtable,
                data: KVMap::new(),
            }
        }
    }

    impl Constructor for MemTableConstructor {
        fn add(&mut self, key: KeyWrapper, value: &[u8]) {
            self.data.insert(key, value.to_vec());
        }

        fn finish_impl(&mut self, options: &Options, data: &KVMap) -> Result<(), DbError> {
            self.memtable = MemTable::new(&self.internal_comparator);
            let mut seq = 1;
            for kvp in &self.data {
                self.memtable
                    .add(seq, TypeValue, kvp.0.key.as_slice(), kvp.1);
                seq += 1;
            }
            Ok(())
        }

        fn new_iterator(&self) -> Box<dyn DBIterator + '_> {
            todo!()
        }

        fn data(&self) -> &KVMap {
            &self.data
        }
    }

    #[derive(Clone, Eq, PartialEq, Debug)]
    enum TestType {
        TABLE,
        BLOCK,
        MEMTABLE,
        DB,
    }

    struct TestArgs {
        r#type: TestType,
        reverse_compare: bool,
        restart_interval: usize,
    }

    const kNumTestArgs: usize = 16;
    const kTestArgList: [TestArgs; kNumTestArgs] = [
        TestArgs {
            r#type: TestType::TABLE,
            reverse_compare: false,
            restart_interval: 16,
        },
        TestArgs {
            r#type: TestType::TABLE,
            reverse_compare: false,
            restart_interval: 1,
        },
        TestArgs {
            r#type: TestType::TABLE,
            reverse_compare: false,
            restart_interval: 1024,
        },
        TestArgs {
            r#type: TestType::TABLE,
            reverse_compare: true,
            restart_interval: 16,
        },
        TestArgs {
            r#type: TestType::TABLE,
            reverse_compare: true,
            restart_interval: 1,
        },
        TestArgs {
            r#type: TestType::TABLE,
            reverse_compare: true,
            restart_interval: 1024,
        },
        TestArgs {
            r#type: TestType::BLOCK,
            reverse_compare: false,
            restart_interval: 16,
        },
        TestArgs {
            r#type: TestType::BLOCK,
            reverse_compare: false,
            restart_interval: 1,
        },
        TestArgs {
            r#type: TestType::BLOCK,
            reverse_compare: false,
            restart_interval: 1024,
        },
        TestArgs {
            r#type: TestType::BLOCK,
            reverse_compare: true,
            restart_interval: 16,
        },
        TestArgs {
            r#type: TestType::BLOCK,
            reverse_compare: true,
            restart_interval: 1,
        },
        TestArgs {
            r#type: TestType::BLOCK,
            reverse_compare: true,
            restart_interval: 1024,
        },
        TestArgs {
            r#type: TestType::MEMTABLE,
            reverse_compare: false,
            restart_interval: 16,
        },
        TestArgs {
            r#type: TestType::MEMTABLE,
            reverse_compare: true,
            restart_interval: 16,
        },
        TestArgs {
            r#type: TestType::DB,
            reverse_compare: false,
            restart_interval: 16,
        },
        TestArgs {
            r#type: TestType::DB,
            reverse_compare: true,
            restart_interval: 16,
        },
    ];

    struct TestTemplate {
        pub options: Options,
        pub construct: Box<dyn Constructor>,
    }

    impl TestTemplate {
        pub fn new(args: &TestArgs) -> Self {
            let mut options = Options::default();

            options.block_restart_interval = args.restart_interval as isize;
            options.block_size = 256;

            if args.reverse_compare {
                options.comparator = Arc::new(Box::new(ReverseKeyComparator {}));
            }

            match args.r#type {
                TestType::TABLE => {
                    let cmp = options.comparator.clone();
                    TestTemplate {
                        options,
                        construct: Box::new(TableConstructor::new(cmp)),
                    }
                }
                TestType::BLOCK => {
                    let cmp = options.comparator.clone();
                    TestTemplate {
                        options,
                        construct: Box::new(BlockConstructor::new(cmp)),
                    }
                }
                TestType::MEMTABLE => {
                    let cmp = options.comparator.clone();
                    TestTemplate {
                        options,
                        construct: Box::new(BlockConstructor::new(cmp)),
                    }
                }
                TestType::DB => {
                    todo!("DB")
                }
            }
        }

        pub fn add(&mut self, key: KeyWrapper, value: &[u8]) {
            self.construct.add(key, value);
        }

        pub fn test(&mut self) {
            let mut keys = Vec::new();
            let mut kvmap = KVMap::new();
            self.construct
                .finish(&self.options, &mut keys, &mut kvmap)
                .unwrap();
            self.test_forward_scan(&keys, &kvmap);
            self.test_backward_scan(&keys, &kvmap);
        }

        pub fn test_forward_scan(&mut self, keys: &Vec<KeyWrapper>, data: &KVMap) {
            let mut iter = self.construct.new_iterator();
            assert!(!iter.valid());
            iter.seek_to_first();
            println!("Forward scan data size {}", data.len());
            for (k, v) in data {
                println!(
                    "start iterator, key: {:?}, value: {:?}",
                    k.key.as_slice(),
                    v
                );
                assert!(iter.valid());
                println!("forward {:?} value: {:?}", k.key.as_slice(), v);
                assert_eq!(iter.key(), k.key.as_slice());
                assert_eq!(iter.value(), v.as_slice());
                iter.next();
            }

            assert!(!iter.valid());
        }

        pub fn test_backward_scan(&mut self, keys: &Vec<KeyWrapper>, data: &KVMap) {
            let mut iter = self.construct.new_iterator();
            assert!(!iter.valid());
            iter.seek_to_last();
            for (k, v) in data.iter().rev() {
                assert_eq!(iter.key(), k.key.as_slice());
                assert_eq!(iter.value(), v.as_slice());
                iter.prev();
            }
            assert!(!iter.valid());
        }

        pub fn test_random_access(&mut self) {
            todo!("Test random access")
        }
    }

    #[test]
    fn test_empty() {
        for i in 0..kNumTestArgs {
            if kTestArgList[i].r#type != TestType::BLOCK
                && kTestArgList[i].r#type != TestType::TABLE
                && kTestArgList[i].r#type != TestType::MEMTABLE
            {
                continue;
            }
            let mut test = TestTemplate::new(&kTestArgList[i]);
            println!("Test empty: {:?}", i);
            test.test();
        }
    }
    #[test]
    // Special test for a block with no restart entries.  The C++ leveldb
    // code never generates such blocks, but the Java version of leveldb
    // seems to.
    fn test_zero_restart_points_in_block() {
        let contents = BlockContent::new(vec![0, 0, 0, 0], false, false);
        let mut block = Block::new(contents);
        let mut iter = block.new_iterator_into(Arc::new(Box::new(BytewiseComparatorImpl)));
        iter.seek_to_first();
        assert!(!iter.valid());
        iter.seek_to_first();
        assert!(!iter.valid());
        iter.seek(b"foo");
        assert!(!iter.valid());
    }

    #[test]
    fn test_simple_empty_key() {
        for i in 0..kNumTestArgs {
            if kTestArgList[i].r#type != TestType::BLOCK
                && kTestArgList[i].r#type != TestType::TABLE
                && kTestArgList[i].r#type != TestType::MEMTABLE
            {
                continue;
            }
            let mut test = TestTemplate::new(&kTestArgList[i]);
            println!("Test simple empty key: {:?}", i);
            let key = KeyWrapper {
                key: Vec::new(),
                cmp: test.options.comparator.clone(),
            };
            test.add(key, b"v");
            test.test();
        }
    }

    #[test]
    fn test_simple_single() {
        for i in 0..kNumTestArgs {
            if kTestArgList[i].r#type != TestType::BLOCK
                && kTestArgList[i].r#type != TestType::TABLE
                && kTestArgList[i].r#type != TestType::MEMTABLE
            {
                continue;
            }
            let mut test = TestTemplate::new(&kTestArgList[i]);
            println!("Test simple single: {:?}", i);
            let key = KeyWrapper {
                key: b"abc".to_vec(),
                cmp: test.options.comparator.clone(),
            };
            test.add(key, b"v");
            test.test();
        }
    }

    #[test]
    fn test_simple_multi() {
        for i in 0..kNumTestArgs {
            if kTestArgList[i].r#type != TestType::BLOCK
                && kTestArgList[i].r#type != TestType::TABLE
                && kTestArgList[i].r#type != TestType::MEMTABLE
            {
                continue;
            }
            let mut test = TestTemplate::new(&kTestArgList[i]);
            println!("Test simple multi: {:?}", i);
            let key1 = KeyWrapper {
                key: b"abc".to_vec(),
                cmp: test.options.comparator.clone(),
            };
            test.add(key1, b"v");
            let key2 = KeyWrapper {
                key: b"abcd".to_vec(),
                cmp: test.options.comparator.clone(),
            };
            test.add(key2, b"v");
            let key3 = KeyWrapper {
                key: b"ac".to_vec(),
                cmp: test.options.comparator.clone(),
            };
            test.add(key3, b"v2");
            test.test();
        }
    }

    #[test]
    fn test_simple_special_key() {
        for i in 0..kNumTestArgs {
            if kTestArgList[i].r#type != TestType::BLOCK
                && kTestArgList[i].r#type != TestType::TABLE
                && kTestArgList[i].r#type != TestType::MEMTABLE
            {
                continue;
            }
            let mut test = TestTemplate::new(&kTestArgList[i]);
            println!("Test simple special key: {:?}", i);
            let key1 = KeyWrapper {
                key: b"\xff\xff".to_vec(),
                cmp: test.options.comparator.clone(),
            };
            test.add(key1, b"v3");
            test.test();
        }
    }

    // this test is too slow, so we skip it
    #[test]
    fn test_randomized() {
        for i in 0..kNumTestArgs {
            println!("======Test randomized: {:?}", kTestArgList[i].r#type);
            if kTestArgList[i].r#type == TestType::DB {
                continue;
            }
            let mut test = TestTemplate::new(&kTestArgList[i]);
            println!("Test randomized: {:?}", i);
            let mut num_entries = 0;
            let mut rng = thread_rng();
            while num_entries < 1000 {
                if (num_entries % 10) == 0 {
                    println!(
                        "case {} of {kNumTestArgs}: num_entries = {num_entries}",
                        i + 1
                    )
                }

                for e in 0..num_entries {
                    let len = skewed(&mut rng, 4);
                    let key = KeyWrapper {
                        key: random_key(&mut rng, len as i32),
                        cmp: test.options.comparator.clone(),
                    };
                    let len2 = skewed(&mut rng, 5);
                    let value = random_string(&mut rng, len2 as i32);
                    test.add(key, value.as_bytes());
                }

                if num_entries < 50 {
                    num_entries += 1;
                } else {
                    num_entries += 200;
                }
                test.test();
            }
        }
    }

    fn between(val: u64, low: u64, high: u64) -> bool {
        let result = (val >= low) && (val <= high);
        if !result {
            println!("Value {} is not in range [{}, {}]", val, low, high);
        }
        result
    }

    #[test]
    fn test_table_approximate_offset_of_plain() {
        let mut c = TableConstructor::new(Arc::new(Box::new(BytewiseComparatorImpl)));
        c.add(
            KeyWrapper {
                key: b"k01".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            b"hello",
        );
        c.add(
            KeyWrapper {
                key: b"k02".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            b"hello2",
        );
        c.add(
            KeyWrapper {
                key: b"k03".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            vec!['x' as u8; 10000].as_slice(),
        );
        c.add(
            KeyWrapper {
                key: b"k04".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            vec!['x' as u8; 200000].as_slice(),
        );
        c.add(
            KeyWrapper {
                key: b"k05".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            vec!['x' as u8; 300000].as_slice(),
        );
        c.add(
            KeyWrapper {
                key: b"k06".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            b"hello3",
        );
        c.add(
            KeyWrapper {
                key: b"k07".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            vec!['x' as u8; 100000].as_slice(),
        );

        let mut options = Options::default();
        options.block_size = 1024;
        options.compression = crate::util::options::CompressionType::NoCompression;

        let mut keys = Vec::new();
        let mut kvmap = KVMap::new();
        c.finish(&options, &mut keys, &mut kvmap).unwrap();

        assert!(between(c.approximate_offset_of_plain(b"k01"), 0, 0));
    }

    fn compress_support(t: &CompressionType) -> bool {
        let in_data = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut out = Vec::new();
        match t {
            CompressionType::SnappyCompression => {
                if let Ok(_) = snap::raw::Encoder::new().compress_vec(in_data.as_bytes()) {
                    true
                } else {
                    false
                }
            }
            CompressionType::ZstdCompression => {
                if let Ok(_) = zstd::stream::copy_encode(in_data.as_bytes(), &mut out, 1) {
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    // rewrite this cpp to rust:   Random rnd(301);
    #[test]
    fn test_table_approximate_offset_of_compressed() {
        let mut c = TableConstructor::new(Arc::new(Box::new(BytewiseComparatorImpl)));
        let mut rnd = thread_rng();
        c.add(
            KeyWrapper {
                key: b"k01".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            b"hello",
        );
        c.add(
            KeyWrapper {
                key: b"k02".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            compressible_string(&mut rnd, 10000, 0.25).as_bytes(),
        );
        c.add(
            KeyWrapper {
                key: b"k03".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            b"hello3",
        );
        c.add(
            KeyWrapper {
                key: b"k04".to_vec(),
                cmp: Arc::new(Box::new(BytewiseComparatorImpl)),
            },
            compressible_string(&mut rnd, 10000, 0.25).as_bytes(),
        );

        let mut compression_types = vec![
            CompressionType::SnappyCompression,
            CompressionType::ZstdCompression,
        ];

        for t in compression_types {
            if !compress_support(&t) {
                continue;
            }

            let mut options = Options::default();
            options.block_size = 1024;
            options.compression = t;

            let mut keys = Vec::new();
            let mut kvmap = KVMap::new();
            c.finish(&options, &mut keys, &mut kvmap).unwrap();

            let expected = 2500;
            let kSlop = 1000;
            let min_z = expected - kSlop;
            let max_z = expected + kSlop;

            assert!(between(c.approximate_offset_of_plain(b"abc"), 0, kSlop));
            assert!(between(c.approximate_offset_of_plain(b"k01"), 0, kSlop));
            assert!(between(c.approximate_offset_of_plain(b"k02"), 0, kSlop));
            assert!(between(c.approximate_offset_of_plain(b"k03"), min_z, max_z));
            assert!(between(c.approximate_offset_of_plain(b"k04"), min_z, max_z));
            assert!(between(
                c.approximate_offset_of_plain(b"xyz"),
                2 * min_z,
                2 * max_z
            ));
        }
    }
}
