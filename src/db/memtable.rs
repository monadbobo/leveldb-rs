use crate::db::dbformat::{InternalKeyComparator, LookupKey, SequenceNumber, ValueType};
use crate::db::error::DbError;
use crate::db::iterator::DBIterator;
use crate::db::skiplist::SkipList;
use crate::util::coding::{
    decode_fixed64, get_varint32, put_varint32, put_varint64, varint_length,
};
use crate::util::comparator::{BytewiseComparatorImpl, Comparator};
use bytes::{Bytes, BytesMut};
use std::cmp::Ordering;
use zstd::zstd_safe::WriteBuf;

type Table = SkipList<Bytes, KeyComparator>;

fn get_length_prefixed_slice(v: &[u8]) -> &[u8] {
    let (bytes_read, len) = get_varint32(&v[5..]).unwrap();
    &v[bytes_read..bytes_read + len as usize]
}

fn encode_key<'a>(scratch: &'a mut Vec<u8>, target: &[u8]) -> &'a Vec<u8> {
    scratch.clear();
    scratch.extend_from_slice(&put_varint32(target.len() as u32));
    scratch.extend_from_slice(target);
    scratch
}

struct MemTableIterator<'a> {
    iter: crate::db::skiplist::Iterator<'a, Bytes, KeyComparator>,
    tmp: Vec<u8>,
}

struct MemTableBackwardIterator;

impl<'a> MemTableIterator<'a> {
    pub fn new(table: &'a Table) -> Self {
        Self {
            iter: table.iter(),
            tmp: Vec::new(),
        }
    }
}
impl DBIterator for MemTableIterator<'_> {
    fn valid(&self) -> bool {
        self.iter.valid()
    }

    fn seek_to_first(&mut self) {
        self.iter.seek_to_first();
    }

    fn seek_to_last(&mut self) {
        self.iter.seek_to_last();
    }

    fn seek(&mut self, k: &[u8]) {
        let encoded = encode_key(&mut self.tmp, k);
        let e = unsafe { std::mem::transmute::<&[u8], &'static [u8]>(encoded.as_slice()) };
        self.iter.seek(&Bytes::from(e));
    }

    fn next(&mut self) {
        self.iter.next();
    }

    fn prev(&mut self) {
        self.iter.prev();
    }

    fn key(&self) -> &[u8] {
        get_length_prefixed_slice(&self.iter.key())
    }

    fn value(&self) -> &[u8] {
        let key_slice = get_length_prefixed_slice(&self.iter.key());
        get_length_prefixed_slice(&key_slice[key_slice.len()..])
    }

    fn status(&self) -> Result<(), DbError> {
        Ok(())
    }
}
struct KeyComparator {
    compactor: InternalKeyComparator,
}

impl Comparator for KeyComparator {
    fn name(&self) -> &str {
        "leveldb.KeyComparator"
    }

    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
        self.compactor.compare(a, b)
    }

    fn find_shortest_separator(&self, start: &[u8], limit: &[u8]) -> Option<Vec<u8>> {
        unimplemented!()
    }

    fn find_short_successor(&self, key: &[u8]) -> Option<Vec<u8>> {
        unimplemented!()
    }
}

impl KeyComparator {
    pub fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
        let a = get_length_prefixed_slice(a);
        let b = get_length_prefixed_slice(b);
        self.compactor.compare(a, b)
    }
}

pub struct MemTable {
    comparator: InternalKeyComparator,
    table: Table,
}

impl MemTable {
    pub fn new(comparator: &InternalKeyComparator) -> Self {
        let table = Table::new(KeyComparator {
            compactor: comparator.clone(),
        });
        Self {
            comparator: comparator.clone(),
            table,
        }
    }
    pub fn add(&mut self, s: SequenceNumber, t: ValueType, key: &[u8], value: &[u8]) {
        // Format of an entry is concatenation of:
        //  key_size     : varint32 of internal_key.size()
        //  key bytes    : char[internal_key.size()]
        //  tag          : uint64((sequence << 8) | type)
        //  value_size   : varint32 of value.size()
        //  value bytes  : char[value.size()]
        let key_size = key.len();
        let value_size = value.len();
        let internal_key_size = key_size + 8;
        let encoded_len = varint_length(internal_key_size as u64)
            + internal_key_size
            + varint_length(value_size as u64)
            + value_size;
        let mut buf = BytesMut::with_capacity(encoded_len);
        buf.extend_from_slice(&put_varint32(internal_key_size as u32));
        buf.extend_from_slice(key);
        buf.extend_from_slice(&put_varint64((s << 8 | t as u64) as u64));
        buf.extend_from_slice(&put_varint32(value_size as u32));
        buf.extend_from_slice(value);
        self.table.insert(Bytes::from(buf));
    }

    pub fn get(&self, key: &LookupKey) -> Result<Option<Vec<u8>>, DbError> {
        let m = key.memtable_key();
        let extended_m = unsafe { std::mem::transmute::<&[u8], &'static [u8]>(m) };
        let mem_key = Bytes::from(extended_m);
        let mut iter = self.table.iter();
        iter.seek(&mem_key);
        if iter.valid() {
            // entry format is:
            //    klength  varint32
            //    userkey  char[klength]
            //    tag      uint64
            //    vlength  varint32
            //    value    char[vlength]
            // Check that it belongs to same user key.  We do not check the
            // sequence number since the Seek() call above should have skipped
            // all entries with overly large sequence numbers.
            let entry = iter.key();
            match get_varint32(&entry) {
                Some((bytes_read, klength)) => {
                    let user_key = &entry[bytes_read..(bytes_read + klength as usize - 8usize)];
                    if self
                        .comparator
                        .user_comparator
                        .compare(user_key, key.user_key())
                        == Ordering::Equal
                    {
                        let tag = decode_fixed64(&entry[bytes_read + klength as usize - 8usize..]);
                        let tag = ValueType::from((tag & 0xff) as u8);
                        match tag {
                            ValueType::TypeValue => {
                                let v = get_length_prefixed_slice(
                                    &entry[bytes_read..bytes_read + klength as usize],
                                );
                                return Ok(Some(v.to_vec()));
                            }
                            _ => return Err(DbError::NotFound("".to_string())),
                        }
                    }
                }
                None => return Err(DbError::Corruption("bad entry in memtable".to_string())),
            }
        }
        Ok(None)
    }
}
