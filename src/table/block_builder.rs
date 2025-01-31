use crate::util::coding::{put_fixed32, put_varint32};
use crate::util::options::Options;
use bytes::{BufMut, Bytes, BytesMut};
use zstd::zstd_safe::WriteBuf;

pub struct BlockBuilder {
    options: Options,
    buffer: BytesMut,
    restarts: Vec<u32>,
    counter: u32,
    finish: bool,
    last_key: Vec<u8>,
}

impl BlockBuilder {
    pub fn new(options: Options) -> Self {
        BlockBuilder {
            options,
            buffer: BytesMut::new(),
            restarts: vec![0],
            counter: 0,
            finish: false,
            last_key: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.restarts.clear();
        self.restarts.push(0);
        self.counter = 0;
        self.finish = false;
        self.last_key.clear();
    }

    pub fn current_size_estimate(&self) -> usize {
        self.buffer.len() + self.restarts.len() * 4 + 4
    }

    pub fn finish(&mut self) -> Bytes {
        println!("finish restarts offset: {:?}", self.buffer.len());
        for r in &self.restarts {
            println!("finish restarts: {:?}", r);
            self.buffer.put_slice(put_fixed32(*r).as_slice());
        }
        println!("finish restarts len: {:?}", self.restarts.len());
        self.buffer
            .put_slice(put_fixed32(self.restarts.len() as u32).as_slice());
        self.finish = true;
        self.buffer.split().freeze()
    }

    pub fn add(&mut self, key: &[u8], value: &[u8]) {
        assert!(!self.finish);
        let mut shared = 0;
        println!("counter: {}", self.counter);
        if self.counter < self.options.block_restart_interval as u32 {
            let min_length = std::cmp::min(key.len(), self.last_key.len());
            while (shared < min_length) && (key[shared] == self.last_key[shared]) {
                shared += 1;
            }
        } else {
            self.restarts.push(self.buffer.len() as u32);
            self.counter = 0;
            println!("add restarts: {:?}", self.restarts.len());
        }

        let non_shared = key.len() - shared;
        println!("shared: {}, non_shared: {}", shared, non_shared);
        // Add "<shared><non_shared><value_size>" to buffer_
        self.buffer
            .put_slice(put_varint32(shared as u32).as_slice());
        self.buffer
            .put_slice(put_varint32(non_shared as u32).as_slice());
        self.buffer
            .put_slice(put_varint32(value.len() as u32).as_slice());
        // Add string delta to buffer_ followed by value
        self.buffer.put_slice(&key[shared..]);
        self.buffer.put_slice(value);

        //update state
        self.last_key.resize(shared, 0);
        self.last_key.append(&mut key[shared..].to_vec());
        self.counter += 1;
    }

    pub fn empty(&self) -> bool {
        self.buffer.is_empty()
    }
}
