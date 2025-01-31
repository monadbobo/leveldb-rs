use crate::db::iterator::{new_error_iterator, DBIterator, EmptyDBIterator};
use crate::table::format::BlockContent;
use crate::util::coding::{decode_fixed32, get_varint32};
use crate::util::comparator::Comparator;
use std::sync::Arc;

#[derive(Debug)]
pub struct Block {
    data: Vec<u8>,
    size: isize,
    restart_offset: u32,
    owned: bool,
}

fn decode_entry(data: &[u8]) -> Option<(&[u8], u32, u32, u32)> {
    if data.len() < 3 {
        return None;
    }
    let mut shared = data[0] as u32;
    let mut non_shared = data[1] as u32;
    let mut value_length = data[2] as u32;
    println!(
        "decode_entry: shared: {}, non_shared: {}, value_length: {}",
        shared, non_shared, value_length
    );
    let mut size = 0;
    if (shared | non_shared | value_length) < 128 {
        println!(
            "shared: {}, non_shared: {}, value_length: {}",
            shared, non_shared, value_length
        );
        size += 3;
    } else {
        let (s, sd) = get_varint32(&data[size..])?;
        shared = sd;
        size += s;
        let (s, nsd) = get_varint32(&data[size as usize..])?;
        non_shared = nsd;
        size += s;
        let (s, vl) = get_varint32(&data[size as usize..])?;
        value_length = vl;
        size += s;
    }

    if data.len() < size as usize + non_shared as usize + value_length as usize {
        return None;
    }
    Some((&data[size..], shared, non_shared, value_length))
}

impl Block {
    fn num_restarts(&self) -> u32 {
        decode_fixed32(&self.data[self.size as usize - 4..])
    }

    pub fn new(block_content: BlockContent) -> Block {
        let mut b = Block {
            size: block_content.data.len() as isize,
            data: block_content.data,
            restart_offset: 0,
            owned: block_content.heap_allocated,
        };

        println!("size: {}", b.size);
        if b.size < 4 {
            b.size = 0;
        } else {
            let max_restarts_allowed = (b.size - 4) / 4;
            if b.num_restarts() > max_restarts_allowed as u32 {
                b.size = 0;
            } else {
                b.restart_offset = b.size as u32 - (b.num_restarts() + 1) * 4;
                println!("restart_offset: {}", b.restart_offset);
                println!("num_restarts: {}", b.num_restarts());
                println!("size: {}", b.size);
            }
        }
        b
    }

    pub fn size(&self) -> isize {
        self.size
    }

    pub fn new_iterator_into(
        mut self,
        comparator: Arc<Box<dyn Comparator>>,
    ) -> Box<dyn DBIterator> {
        if self.size < 4 {
            return new_error_iterator(Err(crate::db::error::DbError::Corruption(
                "bad block contents".to_string(),
            )));
        }

        let num_restarts = self.num_restarts();
        if num_restarts == 0 {
            Box::new(EmptyDBIterator::new(Ok(())))
        } else {
            let owned_value = Box::new(self.data);

            let data: &[u8] = unsafe {
                // Convert the reference of owned_value to 'static,
                // which is safe because we ensure that the lifetime of owned_value outlives BlockIter
                //    let data: &'static [u8] = std::mem::transmute(&owned_value[..]);
                //data
                std::mem::transmute(&owned_value[..])
            };

            let iter = BlockIter {
                comparator,
                data,
                restarts: self.restart_offset,
                num_restarts,
                current: self.restart_offset,
                restart_index: num_restarts,
                next_entry_offset: 0,
                key: Vec::new(),
                value: &[],
                status: Ok(()),
            };
            Box::new(BlockIterInto {
                inner: Box::new(iter),
                owned_value,
            })
        }
    }

    pub fn new_iterator(&self, comparator: Arc<Box<dyn Comparator>>) -> Box<dyn DBIterator + '_> {
        println!("new_iterator");
        if self.size < 4 {
            println!("bad block contents");
            return new_error_iterator(Err(crate::db::error::DbError::Corruption(
                "bad block contents".to_string(),
            )));
        }

        println!(
            "num_restarts: {}, restarts: {}",
            self.num_restarts(),
            self.restart_offset
        );
        let num_restarts = self.num_restarts();
        if num_restarts == 0 {
            println!("EmptyDBIterator");
            Box::new(EmptyDBIterator::new(Ok(())))
        } else {
            Box::new(BlockIter {
                comparator,
                data: &self.data,
                restarts: self.restart_offset,
                num_restarts,
                current: self.restart_offset,
                restart_index: num_restarts,
                next_entry_offset: 0,
                key: Vec::new(),
                value: &[],
                status: Ok(()),
            })
        }
    }
}

struct BlockIterInto<'a> {
    inner: Box<BlockIter<'a>>,
    owned_value: Box<Vec<u8>>,
}

impl BlockIterInto<'_> {
    pub fn compare(&self, a: &[u8], b: &[u8]) -> std::cmp::Ordering {
        self.inner.compare(a, b)
    }

    pub fn next_entry_offset(&self) -> u32 {
        self.inner.next_entry_offset()
    }

    pub fn get_restart_point(&self, index: u32) -> u32 {
        self.inner.get_restart_point(index)
    }

    pub fn seek_to_restart_point(&mut self, index: u32) {
        self.inner.seek_to_restart_point(index);
    }

    fn corruption_error(&mut self) {
        self.inner.corruption_error();
    }

    fn parse_next_key(&mut self) -> bool {
        self.inner.parse_next_key()
    }
}

impl DBIterator for BlockIterInto<'_> {
    fn valid(&self) -> bool {
        self.inner.valid()
    }

    fn seek_to_first(&mut self) {
        self.inner.seek_to_first();
    }

    fn seek_to_last(&mut self) {
        self.inner.seek_to_last();
    }

    fn seek(&mut self, target: &[u8]) {
        self.inner.seek(target);
    }

    fn next(&mut self) {
        self.inner.next();
    }

    fn prev(&mut self) {
        self.inner.prev();
    }

    fn key(&self) -> &[u8] {
        self.inner.key()
    }

    fn value(&self) -> &[u8] {
        self.inner.value()
    }

    fn status(&self) -> Result<(), crate::db::error::DbError> {
        self.inner.status()
    }
}

struct BlockIter<'a> {
    comparator: Arc<Box<dyn Comparator>>,
    data: &'a [u8],
    restarts: u32,
    num_restarts: u32,

    current: u32,
    restart_index: u32,
    next_entry_offset: u32,
    key: Vec<u8>,
    value: &'a [u8],

    status: Result<(), crate::db::error::DbError>,
}

impl BlockIter<'_> {
    pub fn compare(&self, a: &[u8], b: &[u8]) -> std::cmp::Ordering {
        self.comparator.compare(a, b)
    }

    pub fn next_entry_offset(&self) -> u32 {
        self.next_entry_offset
    }

    pub fn get_restart_point(&self, index: u32) -> u32 {
        assert!(index < self.num_restarts);
        decode_fixed32(&self.data[(self.restarts + index * 4) as usize..])
    }

    pub fn seek_to_restart_point(&mut self, index: u32) {
        self.key.clear();
        self.restart_index = index;

        let offset = self.get_restart_point(index);
        //let slice = self.data.as_ref();
        //self.value = &slice[offset as usize..];
        self.value = &self.data[offset as usize..];
        self.next_entry_offset = offset;
    }

    fn corruption_error(&mut self) {
        self.current = self.restarts;
        self.restart_index = self.num_restarts;
        self.key.clear();
        self.value = &[];
        self.status = Err(crate::db::error::DbError::Corruption(
            "bad entry in block".to_string(),
        ));
    }

    fn parse_next_key(&mut self) -> bool {
        self.current = self.next_entry_offset();
        println!("parse_next_key: current: {}", self.current);
        if self.current >= self.restarts {
            self.current = self.restarts;
            self.restart_index = self.num_restarts;
            println!("current: {}, restarts: {}", self.current, self.restarts);
            return false;
        }
        // Decode next entry
        let entry = decode_entry(&self.data[self.current as usize..]);
        if entry.is_none() || self.key.len() < entry.unwrap().1 as usize {
            println!("key len: {}", self.key.len());
            println!("entry: {:?}", entry);
            self.corruption_error();
            false
        } else {
            let (key, shared, non_shared, value_length) = entry.unwrap();
            println!(
                "{:?}, {:?}, {:?}, {:?}, {:?}",
                key,
                shared,
                non_shared,
                value_length,
                (self.data.len() - key.len()) as u32 - self.current
            );
            self.key.resize(shared as usize, 0);
            self.key.extend_from_slice(&key[..non_shared as usize]);
            self.value = &key[non_shared as usize..(non_shared + value_length) as usize];
            self.next_entry_offset +=
                non_shared + value_length + (self.data.len() - key.len()) as u32 - self.current;
            println!("next_entry_offset: {}", self.next_entry_offset);

            while self.restart_index + 1 < self.num_restarts
                && self.get_restart_point(self.restart_index + 1) < self.current
            {
                self.restart_index += 1;
            }
            true
        }
    }
}

impl DBIterator for BlockIter<'_> {
    fn valid(&self) -> bool {
        println!("current: {}, restarts: {}", self.current, self.restarts);
        self.current < self.restarts
    }

    fn seek_to_first(&mut self) {
        println!("seek_to_first");
        self.seek_to_restart_point(0);
        self.parse_next_key();
        println!("seek_to_first valid: {}", self.valid());
    }

    fn seek_to_last(&mut self) {
        self.seek_to_restart_point(self.num_restarts - 1);
        while self.parse_next_key() && self.next_entry_offset() < self.restarts {}
    }

    fn seek(&mut self, target: &[u8]) {
        let mut left = 0_u32;
        let mut right = self.num_restarts - 1;
        let mut current_key_compare = std::cmp::Ordering::Equal;

        if self.valid() {
            current_key_compare = self.compare(self.key(), target);
            if current_key_compare == std::cmp::Ordering::Less {
                left = self.restart_index;
            } else if current_key_compare == std::cmp::Ordering::Greater {
                right = self.restart_index;
            } else {
                return;
            }
        }

        while left < right {
            let mid = (left + right + 1) / 2;
            let region_offset = self.get_restart_point(mid);
            let entry = decode_entry(&self.data[region_offset as usize..]);
            if entry.is_none() || (entry.unwrap().1 != 0) {
                self.corruption_error();
                return;
            }

            let (key, shared, non_shared, _) = entry.unwrap();
            let mid_key = &key[..non_shared as usize];

            if self.compare(mid_key, target) == std::cmp::Ordering::Less {
                left = mid;
            } else {
                right = mid - 1;
            }
        }

        let skip_seek =
            left == self.restart_index && current_key_compare == std::cmp::Ordering::Less;
        if !skip_seek {
            self.seek_to_restart_point(left);
        }

        loop {
            if !self.parse_next_key() {
                break;
            }
            if self.compare(self.key(), target) != std::cmp::Ordering::Less {
                break;
            }
        }
    }

    fn next(&mut self) {
        assert!(self.valid());
        self.parse_next_key();
    }

    fn prev(&mut self) {
        assert!(self.valid());
        let origin = self.current;
        while self.get_restart_point(self.restart_index) >= origin {
            if self.restart_index == 0 {
                self.current = self.restarts;
                self.restart_index = self.num_restarts;
                return;
            }
            self.restart_index -= 1;
        }

        self.seek_to_restart_point(self.restart_index);
        while self.parse_next_key() && self.next_entry_offset() < origin {}
    }

    fn key(&self) -> &[u8] {
        self.key.as_slice()
    }

    fn value(&self) -> &[u8] {
        self.value
    }

    fn status(&self) -> Result<(), crate::db::error::DbError> {
        //        &self.status
        Ok(())
    }
}
