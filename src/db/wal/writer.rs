use std::{io, io::Write};

use bytes::{Buf, BufMut, BytesMut};
use crc32fast::Hasher;
use tracing::{debug, error};

use crate::db::wal::format::{RecordType, BLOCK_SIZE, HEADER_SIZE, MAX_RECORD_TYPE};

#[derive(Debug)]
pub struct Writer {
    dest: std::fs::File,
    block_offset: usize,
    type_crc: [u32; 5],
    pub size: usize,
}

fn init_type_crc() -> [u32; 5] {
    let mut v: [u32; 5] = [0; 5];
    for i in 0..=*MAX_RECORD_TYPE {
        let mut hasher = Hasher::new();
        hasher.update(&[i]);
        v[i as usize] = hasher.finalize();
    }
    v
}

impl Writer {
    pub fn new(dest: std::fs::File, dest_length: u64) -> Writer {
        Writer {
            dest,
            block_offset: (dest_length % BLOCK_SIZE) as usize,
            type_crc: init_type_crc(),
            size: dest_length as usize,
        }
    }

    pub fn add_record(&mut self, data: &mut BytesMut) -> io::Result<()> {
        let mut begin = true;
        let mut left = data.len();

        debug!("add record data length: {}", left);

        loop {
            let leftover = BLOCK_SIZE - self.block_offset as u64;
            if leftover < HEADER_SIZE as u64 {
                if leftover > 0 {
                    let _ = self.dest.write(&vec![0; leftover as usize]);
                }

                self.block_offset = 0;
            }

            let avail = BLOCK_SIZE as usize - self.block_offset - HEADER_SIZE;
            let fragment_length = if left < avail { left } else { avail };

            let end = left == fragment_length;

            let record_type = if begin && end {
                RecordType::Full
            } else if begin {
                RecordType::First
            } else if end {
                RecordType::Last
            } else {
                RecordType::Middle
            };

            let result = match self.emit_physical_record(record_type, &data[..fragment_length]) {
                Ok(_) => Ok(()),
                Err(e) => {
                    error!("emit record error: {}", e);
                    Err(e)
                }
            };

            data.advance(fragment_length);
            left -= fragment_length;
            begin = false;

            if left == 0 || result.is_err() {
                return result;
            }
        }
    }

    fn emit_physical_record(&mut self, t: RecordType, data: &[u8]) -> io::Result<()> {
        let t_num = t as u8;
        let mut buf = BytesMut::with_capacity(HEADER_SIZE);

        let mut hasher = Hasher::new_with_initial(self.type_crc[t_num as usize]);
        hasher.update(data);

        let crc = hasher.finalize();
        buf.put_u32(crc);
        buf.put_u8((data.len() & 0xff) as u8);
        buf.put_u8((data.len() >> 8) as u8);
        buf.put_u8(t_num);

        debug!(
            "emit_physical_record: {},{},{}",
            data.len(),
            t_num,
            self.type_crc[t_num as usize]
        );

        let mut result = io::Result::Ok(());
        match self.dest.write_all(&buf[..]) {
            Ok(_) => {
                self.size += buf.len();
                match self.dest.write_all(data) {
                    Ok(_) => {
                        self.size += data.len();
                        debug!("write data success");
                        let _ = self.dest.flush();
                    }
                    Err(e) => {
                        error!("write data error: {}", e);
                        result = Err(e);
                    }
                }
            }
            Err(e) => {
                error!("write header error: {}", e);
                result = Err(e);
            }
        }

        self.block_offset += HEADER_SIZE + data.len();
        result
    }
}
