pub mod format;
pub mod reader;
pub mod writer;

pub fn make_log_file_name(log_number: u64) -> String {
    format!("{log_number:0>6}.log")
}

#[cfg(test)]
mod test {
    use std::{
        fs,
        fs::{File, OpenOptions},
    };

    use bytes::BytesMut;
    use rand::Rng;

    use crate::db::wal::{
        format::{BLOCK_SIZE, HEADER_SIZE},
        reader::Reader,
        writer::Writer,
    };

    #[test]
    fn test_simple_wal() {
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("/tmp/mencius_wal.log")
            .unwrap();

        let mut writer = Writer::new(f, 0);
        let v1 = vec![1_u8, 2, 3, 4];
        let mut b = BytesMut::new();
        b.extend_from_slice(&v1);
        writer.add_record(&mut b).unwrap();

        let v2 = vec![5_u8, 6, 7, 8];
        let mut b = BytesMut::new();
        b.extend_from_slice(&v2);
        writer.add_record(&mut b).unwrap();

        let f_read = File::open("/tmp/mencius_wal.log").unwrap();

        let mut reader = Reader::new(f_read, true, 0);
        let mut scratch = BytesMut::with_capacity(BLOCK_SIZE as usize);

        let record = reader.read_record(&mut scratch).unwrap();
        let v3 = &record[..];
        assert_eq!(*v1, *v3);

        let record = reader.read_record(&mut scratch).unwrap();
        let v4 = &record[..];
        assert_eq!(*v2, *v4);

        fs::remove_file("/tmp/mencius_wal.log").unwrap();
    }

    #[test]
    fn test_complex_wal() {
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("/tmp/mencius_wal_complex.log")
            .unwrap();

        let mut writer = Writer::new(f, 0);
        let mut v = Vec::new();
        for _ in 0..BLOCK_SIZE * 2 {
            let n1: u8 = rand::thread_rng().gen_range(0..255);
            v.push(n1);
        }

        let mut b = BytesMut::new();
        b.extend_from_slice(&v);
        writer.add_record(&mut b).unwrap();

        let v2 = vec![1_u8; (BLOCK_SIZE * 2) as usize];
        let mut b = BytesMut::new();
        b.extend_from_slice(&v2);
        writer.add_record(&mut b).unwrap();

        let f_read = File::open("/tmp/mencius_wal_complex.log").unwrap();

        let mut reader = Reader::new(f_read, true, 0);
        let mut scratch = BytesMut::with_capacity(BLOCK_SIZE as usize);

        let record = reader.read_record(&mut scratch).unwrap();
        let v3 = &record[..];
        assert_eq!(*v, *v3);

        let record = reader.read_record(&mut scratch).unwrap();
        let v4 = &record[..];
        assert_eq!(*v2, *v4);

        fs::remove_file("/tmp/mencius_wal_complex.log").unwrap();
    }

    #[test]
    fn test_trailer_wal() {
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("/tmp/mencius_wal_trailer.log")
            .unwrap();

        let mut writer = Writer::new(f, 0);
        let mut v = Vec::new();
        for _ in 0..(BLOCK_SIZE as usize - HEADER_SIZE - 3) {
            let n1: u8 = rand::thread_rng().gen_range(0..255);
            v.push(n1);
        }
        let mut b = BytesMut::new();
        b.extend_from_slice(&v);
        writer.add_record(&mut b).unwrap();

        let v2 = vec![22_u8; 100];
        b.clear();
        b.extend_from_slice(&v2);
        writer.add_record(&mut b).unwrap();

        let f_read = File::open("/tmp/mencius_wal_trailer.log").unwrap();

        let mut reader = Reader::new(f_read, true, 0);
        let mut scratch = BytesMut::with_capacity(BLOCK_SIZE as usize);

        let record = reader.read_record(&mut scratch).unwrap();
        let v3 = &record[..];
        assert_eq!(*v, *v3);

        let record = reader.read_record(&mut scratch).unwrap();
        let v4 = &record[..];
        assert_eq!(*v2, *v4);

        fs::remove_file("/tmp/mencius_wal_trailer.log").unwrap();
    }
}
