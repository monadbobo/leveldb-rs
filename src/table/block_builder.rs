use crate::util::coding::{put_fixed32, put_varint32};
use crate::util::options::Options;

#[derive(Debug)]
pub struct BlockBuilder {
    options: Options,
    buffer: Vec<u8>,
    restarts: Vec<u32>,
    counter: u32,
    finish: bool,
    last_key: Vec<u8>,
}

impl BlockBuilder {
    pub fn new(options: Options) -> Self {
        BlockBuilder {
            options,
            buffer: Vec::new(),
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

    pub fn finish(&mut self) -> &[u8] {
        for r in &self.restarts {
            self.buffer.append(&mut put_fixed32(*r));
        }
        self.buffer
            .append(&mut put_fixed32(self.restarts.len() as u32));
        self.finish = true;
        &self.buffer
    }

    pub fn add(&mut self, key: &[u8], value: &[u8]) {
        assert!(!self.finish);
        let mut shared = 0;
        if self.counter < self.options.block_restart_interval as u32 {
            let min_length = std::cmp::min(key.len(), self.last_key.len());
            while (shared < min_length) && (key[shared] == self.last_key[shared]) {
                shared += 1;
            }
        } else {
            self.restarts.push(self.buffer.len() as u32);
            self.counter = 0;
        }

        let non_shared = key.len() - shared;
        // Add "<shared><non_shared><value_size>" to buffer_
        self.buffer.append(&mut put_varint32(shared as u32));
        self.buffer.append(&mut put_varint32(non_shared as u32));
        self.buffer.append(&mut put_varint32(value.len() as u32));
        // Add string delta to buffer_ followed by value
        self.buffer.extend_from_slice(&key[shared..]);
        self.buffer.extend_from_slice(value);

        //update state
        self.last_key.resize(shared, 0);
        self.last_key.append(&mut key[shared..].to_vec());
        self.counter += 1;
    }
}
