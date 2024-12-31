use crate::util::coding::{decode_fixed32, put_fixed32};
use crate::util::filter_policy::FilterPolicy;
use std::sync::Arc;
use zstd::zstd_safe::WriteBuf;

const FILTER_BASE_LG: usize = 11;
const FILTER_BASE: usize = 1 << FILTER_BASE_LG;

pub struct FilterBlockBuilder {
    policy: Arc<Box<dyn FilterPolicy>>,
    keys: Vec<u8>,
    start: Vec<usize>,
    result: Vec<u8>,
    filter_offsets: Vec<u32>,
}

impl FilterBlockBuilder {
    pub fn new(policy: Arc<Box<dyn FilterPolicy>>) -> Self {
        FilterBlockBuilder {
            policy,
            keys: Vec::new(),
            start: Vec::new(),
            result: Vec::new(),
            filter_offsets: Vec::new(),
        }
    }

    pub fn start_block(&mut self, block_offset: u64) {
        let filter_index = block_offset / FILTER_BASE as u64;
        assert!(filter_index >= self.filter_offsets.len() as u64);
        while filter_index > self.filter_offsets.len() as u64 {
            self.generate_filter();
        }
    }

    pub fn add_key(&mut self, key: &[u8]) {
        self.start.push(self.keys.len());
        self.keys.extend_from_slice(key);
    }

    pub fn finish(&mut self) -> &[u8] {
        if !self.start.is_empty() {
            self.generate_filter();
        }

        let array_offset = self.result.len();
        for f in &self.filter_offsets {
            self.result.append(&mut put_fixed32(*f));
        }
        self.result.append(&mut put_fixed32(array_offset as u32));
        self.result.push(FILTER_BASE_LG as u8);
        self.result.as_slice()
    }

    pub fn generate_filter(&mut self) {
        let num_keys = self.start.len();
        if num_keys == 0 {
            // Fast path if there are no keys for this filter
            self.filter_offsets.push(self.result.len() as u32);
            return;
        }
        // Make list of keys from flattened key structure
        self.start.push(self.keys.len());
        let mut tmp_keys = Vec::with_capacity(num_keys);

        for i in 0..num_keys {
            let start = self.start[i];
            let end = self.start[i + 1];
            tmp_keys.push(&self.keys[start..end]);
        }

        self.filter_offsets.push(self.result.len() as u32);
        self.result
            .append(&mut self.policy.create_filter(&tmp_keys));
        self.keys.clear();
        self.start.clear();
    }
}
pub struct FilterBlockReader<'a> {
    policy: Arc<Box<dyn FilterPolicy>>,
    data: Option<&'a [u8]>,
    offset: usize,
    num: usize,
    base_lg: usize,
}

impl<'a> FilterBlockReader<'a> {
    pub fn new(policy: Arc<Box<dyn FilterPolicy>>, data: &'a [u8]) -> Self {
        let mut fb = FilterBlockReader {
            policy,
            data: None,
            offset: 0,
            num: 0,
            base_lg: 0,
        };
        let n = data.len();
        if n < 5 {
            return fb;
        }
        fb.base_lg = data[n - 1] as usize;
        let last_word = decode_fixed32(&data[n - 5..]);
        if last_word > (n - 5) as u32 {
            return fb;
        }

        fb.data = Some(data);
        fb.offset = last_word as usize;
        fb.num = (((n - 5) as u32 - last_word) / 4) as usize;
        fb
    }

    pub fn key_may_match(&self, block_offset: u64, key: &[u8]) -> bool {
        let index = block_offset as usize >> self.base_lg;
        if index < self.num {
            let start = decode_fixed32(&self.data.unwrap()[self.offset + index * 4..]);
            let limit = decode_fixed32(&self.data.unwrap()[self.offset + (index + 1) * 4..]);
            if start <= limit && limit <= self.offset as u32 {
                let filter = &self.data.unwrap()[start as usize..limit as usize];
                return self.policy.key_may_match(key, filter);
            } else if start == limit {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod test {
    use crate::util;
    use crate::util::coding::{decode_fixed32, encode_fixed32};
    use crate::util::filter_policy::FilterPolicy;
    use std::arch::aarch64::vbic_s8;
    use std::sync::Arc;

    struct TestHashFilterPolicy;

    impl FilterPolicy for TestHashFilterPolicy {
        fn name(&self) -> &str {
            "TestHashFilterPolicy"
        }

        fn create_filter(&self, keys: &Vec<&[u8]>) -> Vec<u8> {
            let mut filter = Vec::new();
            for key in keys {
                let h = util::hash::hash(key, 1);
                filter.extend_from_slice(&mut encode_fixed32(h));
            }
            filter
        }

        fn key_may_match(&self, key: &[u8], filter: &[u8]) -> bool {
            let h = util::hash::hash(key, 1);
            for f in filter.chunks_exact(4) {
                if decode_fixed32(f) == h {
                    return true;
                }
            }
            false
        }
    }

    #[test]
    fn test_empty_filter_block() {
        let policy: Arc<Box<dyn FilterPolicy>> = Arc::new(Box::new(TestHashFilterPolicy));
        let mut builder = super::FilterBlockBuilder::new(policy.clone());
        let block = builder.finish();
        assert_eq!(block.len(), 5);
        assert_eq!(block, [0, 0, 0, 0, 11]);
        let reader = super::FilterBlockReader::new(policy.clone(), block);
        assert!(reader.key_may_match(0, b"foo"));
        assert!(reader.key_may_match(100000, b"foo"));
    }

    #[test]
    fn test_single_chunk() {
        let policy: Arc<Box<dyn FilterPolicy>> = Arc::new(Box::new(TestHashFilterPolicy));
        let mut builder = super::FilterBlockBuilder::new(policy.clone());
        builder.start_block(100);
        builder.add_key(b"foo");
        builder.add_key(b"bar");
        builder.add_key(b"box");
        builder.start_block(200);
        builder.add_key(b"box");
        builder.start_block(300);
        builder.add_key(b"hello");
        let block = builder.finish();
        let reader = super::FilterBlockReader::new(policy.clone(), block);
        assert!(reader.key_may_match(100, b"foo"));
        assert!(reader.key_may_match(100, b"bar"));
        assert!(reader.key_may_match(100, b"box"));
        assert!(reader.key_may_match(100, b"hello"));
        assert!(reader.key_may_match(100, b"foo"));
        assert!(!reader.key_may_match(100, b"missing"));
        assert!(!reader.key_may_match(100, b"other"));
    }

    #[test]
    fn test_multi_chunk() {
        let policy: Arc<Box<dyn FilterPolicy>> = Arc::new(Box::new(TestHashFilterPolicy));
        let mut builder = super::FilterBlockBuilder::new(policy.clone());
        builder.start_block(0);
        builder.add_key(b"foo");
        builder.start_block(2000);
        builder.add_key(b"bar");

        builder.start_block(3100);
        builder.add_key(b"box");

        builder.start_block(9000);
        builder.add_key(b"box");
        builder.add_key(b"hello");

        let block = builder.finish();
        let reader = super::FilterBlockReader::new(policy.clone(), block);
        assert!(reader.key_may_match(0, b"foo"));
        assert!(reader.key_may_match(2000, b"bar"));
        assert!(!reader.key_may_match(0, b"box"));
        assert!(!reader.key_may_match(0, b"hello"));

        assert!(reader.key_may_match(3100, b"box"));
        assert!(!reader.key_may_match(3100, b"foo"));
        assert!(!reader.key_may_match(3100, b"bar"));
        assert!(!reader.key_may_match(3100, b"hello"));

        assert!(!reader.key_may_match(4100, b"foo"));
        assert!(!reader.key_may_match(4100, b"bar"));
        assert!(!reader.key_may_match(4100, b"box"));
        assert!(!reader.key_may_match(4100, b"hello"));

        assert!(reader.key_may_match(9000, b"box"));
        assert!(reader.key_may_match(9000, b"hello"));
        assert!(!reader.key_may_match(9000, b"foo"));
        assert!(!reader.key_may_match(9000, b"bar"));
    }
}
