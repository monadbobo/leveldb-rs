use std::{
    convert::TryInto,
    io::{ErrorKind, Read, Seek, SeekFrom},
};

use bytes::{Buf, BufMut, BytesMut};
use crc32fast::Hasher;
use thiserror::Error;
use tracing::{debug, error};

use crate::db::wal::{
    format::{
        RecordType,
        RecordType::{Last, Middle},
        BLOCK_SIZE, HEADER_SIZE,
    },
    reader::ReadError::{BadRecord, UnknownRecordType},
};

#[derive(Debug)]
pub struct Reader {
    file: std::fs::File,
    checksum: bool,
    buffer: BytesMut,
    back_store: Vec<u8>,
    eof: bool,
    pub last_record_offset: u64,
    end_of_buffer_offset: u64,
    initial_offset: u64,
    re_syncing: bool,
}

#[derive(Error, Debug, Clone, PartialEq)]
pub enum ReadError {
    #[error("read to eof")]
    Eof,
    #[error("bad record error: {0}")]
    BadRecord(String),
    #[error("unknown record type: {0}")]
    UnknownRecordType(u8),
}

impl Reader {
    pub fn new(file: std::fs::File, checksum: bool, initial_offset: u64) -> Reader {
        Reader {
            file,
            checksum,
            buffer: Default::default(),
            back_store: vec![0_u8; BLOCK_SIZE as usize],
            eof: false,
            last_record_offset: 0,
            end_of_buffer_offset: 0,
            initial_offset,
            re_syncing: initial_offset > 0,
        }
    }

    fn read_exact(&mut self) -> Result<(), ReadError> {
        loop {
            match self.file.read(&mut self.back_store[..]) {
                Ok(size) => {
                    debug!("read size: {}", size);

                    self.buffer.put_slice(&self.back_store[..size]);
                    self.end_of_buffer_offset += self.buffer.len() as u64;

                    if self.buffer.len() < BLOCK_SIZE as usize {
                        self.eof = true;
                    }

                    break;
                }

                Err(e) if e.kind() == ErrorKind::Interrupted => {
                    continue;
                }

                Err(e) => {
                    error!("read size error: {:?}", e);
                    self.buffer.clear();
                    self.eof = true;
                    return Err(ReadError::Eof);
                }
            }
        }
        Ok(())
    }

    fn read_physical_record(&mut self, result: &mut BytesMut) -> Result<RecordType, ReadError> {
        loop {
            if self.buffer.len() < HEADER_SIZE {
                self.buffer.clear();

                if self.eof {
                    return Err(ReadError::Eof);
                }

                self.read_exact()?;
                continue;
            }

            let (record_type, length, header) = {
                let header = &self.buffer[..];
                let a = header[4] as u32 & 0xff;
                let b = header[5] as u32 & 0xff;
                let record_type_num = header[6];
                if let Ok(record_type) = RecordType::try_from(record_type_num) {
                    let length = a | (b << 8);
                    (record_type, length, header)
                } else {
                    self.buffer.clear();
                    return Err(UnknownRecordType(record_type_num));
                }
            };

            if HEADER_SIZE + length as usize > self.buffer.len() {
                self.buffer.clear();
                if !self.eof {
                    return Err(ReadError::BadRecord("bad record length".to_string()));
                }

                return Err(ReadError::Eof);
            }

            if record_type == RecordType::Zero && length == 0 {
                self.buffer.clear();
                return Err(ReadError::BadRecord("zero length record".to_string()));
            }

            if self.checksum {
                let expected_crc = u32::from_be_bytes(header[..4].try_into().unwrap());
                let mut hasher = Hasher::new();
                hasher.update(&header[(HEADER_SIZE - 1)..(length as usize + HEADER_SIZE)]);
                let actual_crc = hasher.finalize();

                debug!(
                    "checksum: {}.{}.{}.{}",
                    actual_crc, expected_crc, length, header[6]
                );

                if expected_crc != actual_crc {
                    self.buffer.clear();
                    return Err(BadRecord("checksum mismatch".to_string()));
                }
            }

            self.buffer.advance(HEADER_SIZE);
            result.extend_from_slice(&self.buffer[..length as usize]);
            self.buffer.advance(length as usize);

            if self.end_of_buffer_offset
                - self.buffer.len() as u64
                - HEADER_SIZE as u64
                - (length as u64)
                < self.initial_offset
            {
                result.clear();
                return Err(ReadError::BadRecord("".to_string()));
            }

            return Ok(record_type);
        }
    }

    fn skip_to_initial_block(&mut self) -> bool {
        let offset_in_block = (self.initial_offset % BLOCK_SIZE) as isize;
        let mut block_start_location = self.initial_offset - offset_in_block as u64;

        // Don't search a block if we'd be in the trailer
        if offset_in_block > (BLOCK_SIZE as isize - 6) {
            block_start_location += BLOCK_SIZE;
        }

        self.end_of_buffer_offset = block_start_location;

        // Skip to start of first block that can contain the initial record
        if block_start_location > 0 {
            match self
                .file
                .seek(SeekFrom::Current(block_start_location as i64))
            {
                Ok(_) => {}
                Err(e) => {
                    error!("seek error: {}", e);
                    return false;
                }
            }
        }

        true
    }

    pub fn read_record(&mut self, scratch: &mut BytesMut) -> Option<BytesMut> {
        if self.last_record_offset < self.initial_offset && !self.skip_to_initial_block() {
            return None;
        }

        scratch.clear();

        let mut in_fragmented_record = false;
        let mut prospective_record_offset = 0_u64;

        loop {
            let mut fragment = BytesMut::new();

            match self.read_physical_record(&mut fragment) {
                Ok(record_type) => {
                    debug!(
                        "offset: {}, buffer length: {}, fragment: {}, record_type: {}",
                        self.end_of_buffer_offset,
                        self.buffer.len(),
                        fragment.len(),
                        record_type
                    );

                    let physical_record_offset = self.end_of_buffer_offset
                        - self.buffer.len() as u64
                        - HEADER_SIZE as u64
                        - fragment.len() as u64;

                    if self.re_syncing {
                        if record_type == Middle {
                            continue;
                        } else if record_type == Last {
                            self.re_syncing = false;
                            continue;
                        } else {
                            self.re_syncing = false;
                        }
                    }

                    match record_type {
                        RecordType::Zero => {}
                        RecordType::Full => {
                            prospective_record_offset = physical_record_offset;
                            scratch.clear();
                            self.last_record_offset = prospective_record_offset;
                            return Some(fragment);
                        }
                        RecordType::First => {
                            prospective_record_offset = physical_record_offset;
                            scratch.extend(fragment);
                            in_fragmented_record = true;
                        }
                        RecordType::Middle => {
                            if !in_fragmented_record {
                                error!("missing start of fragmented record(1)");
                            } else {
                                scratch.extend(fragment);
                            }
                        }
                        RecordType::Last => {
                            if !in_fragmented_record {
                                error!("missing start of fragmented record(2)");
                            } else {
                                scratch.extend(fragment);
                                self.last_record_offset = prospective_record_offset;
                                return Some(BytesMut::from(&scratch[..]));
                            }
                        }
                    }
                }
                Err(e) => match e {
                    ReadError::Eof => {
                        debug!("read eof: {}", e);
                        if in_fragmented_record {
                            scratch.clear();
                        }

                        return None;
                    }
                    BadRecord(_) => {
                        if in_fragmented_record {
                            in_fragmented_record = false;
                            scratch.clear();
                        }
                        error!("{}", e);
                    }
                    ReadError::UnknownRecordType(_) => {
                        error!("{}", e);
                        in_fragmented_record = false;
                        scratch.clear();
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use bytes::{Buf, BufMut, BytesMut};
    use tempfile::NamedTempFile;

    use crate::db::{
        wal::{
            format::{RecordType, BLOCK_SIZE, HEADER_SIZE, MAX_RECORD_TYPE},
            reader::Reader,
            writer::Writer,
        },
    };

    #[test]
    fn test_unknown_record_type() {
        let mut file = NamedTempFile::new().unwrap();

        let data: [u8; 100] = [1; 100];
        let mut buf = BytesMut::with_capacity(HEADER_SIZE);
        buf.put_u32(0);
        buf.put_u8((data.len() & 0xff) as u8);
        buf.put_u8((data.len() >> 8) as u8);
        // unknown record type
        buf.put_u8(*MAX_RECORD_TYPE + 1);
        assert_eq!(file.write(&buf[..]).unwrap(), HEADER_SIZE);
        assert_eq!(file.write(&data).unwrap(), data.len());
        file.flush().unwrap();

        let read_file = file.reopen().unwrap();
        let mut reader = Reader::new(read_file, false, 0);
        let mut scratch = BytesMut::with_capacity(BLOCK_SIZE as usize);
        assert!(reader.read_record(&mut scratch).is_none());
    }

    #[test]
    fn test_read_simple_record() {
        let file = NamedTempFile::new().unwrap();

        let mut data = BytesMut::with_capacity(100);

        for i in 0..100 {
            data.put_u8(i);
        }

        let write_file = file.reopen().unwrap();
        let mut writer = Writer::new(write_file, 0);
        writer.add_record(&mut data).unwrap();

        let read_file = file.reopen().unwrap();
        let mut reader = Reader::new(read_file, false, 0);
        let mut scratch = BytesMut::with_capacity(BLOCK_SIZE as usize);
        let mut data = reader.read_record(&mut scratch).unwrap();
        for i in 0..100 {
            assert_eq!(i, data.get_u8());
        }
    }

    #[test]
    fn test_read_complex_record() {
        let file = NamedTempFile::new().unwrap();

        let mut data = BytesMut::with_capacity(100);
        let size = (BLOCK_SIZE / 4) + 100;

        for i in 0..size {
            data.put_u64(i);
        }

        let write_file = file.reopen().unwrap();
        let mut writer = Writer::new(write_file, 0);
        writer.add_record(&mut data).unwrap();

        let read_file = file.reopen().unwrap();
        let mut reader = Reader::new(read_file, false, 0);
        let mut scratch = BytesMut::with_capacity(BLOCK_SIZE as usize);
        let mut data = reader.read_record(&mut scratch).unwrap();
        for i in 0..size {
            assert_eq!(i, data.get_u64());
        }
    }

    #[test]
    fn test_skip_bad_record() {
        let mut file = NamedTempFile::new().unwrap();

        let data: [u8; 100] = [1; 100];
        let mut buf = BytesMut::with_capacity(HEADER_SIZE);
        buf.put_u32(0);
        buf.put_u8((data.len() & 0xff) as u8);
        buf.put_u8((data.len() >> 8) as u8);
        // unknown record type
        buf.put_u8(*MAX_RECORD_TYPE + 30);
        assert_eq!(file.write(&buf[..]).unwrap(), HEADER_SIZE);
        assert_eq!(file.write(&data).unwrap(), data.len());
        file.flush().unwrap();

        let data: [u8; 200] = [1; 200];
        let mut buf = BytesMut::with_capacity(HEADER_SIZE);
        let bad_len = BLOCK_SIZE + 100;
        buf.put_u32(0);
        buf.put_u8((bad_len & 0xff) as u8);
        buf.put_u8((bad_len >> 8) as u8);
        // unknown record type
        buf.put_u8(RecordType::Full as u8);
        assert_eq!(file.write(&buf[..]).unwrap(), HEADER_SIZE);
        assert_eq!(file.write(&data).unwrap(), data.len());
        file.flush().unwrap();

        let mut data = BytesMut::with_capacity(30);

        for i in 0..200 {
            data.put_u8(i);
        }
        let write_file = file.reopen().unwrap();
        let mut writer = Writer::new(write_file, 0);
        writer.add_record(&mut data).unwrap();

        let read_file = file.reopen().unwrap();
        let mut reader = Reader::new(read_file, false, 0);

        let mut scratch = BytesMut::with_capacity(BLOCK_SIZE as usize);
        // will skipped the bad record
        let mut data = reader.read_record(&mut scratch).unwrap();
        for i in 0..30 {
            assert_eq!(i, data.get_u8());
        }
    }
}
