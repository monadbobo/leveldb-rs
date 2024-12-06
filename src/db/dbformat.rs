use crate::util::coding::{decode_fixed64, encode_varint32, put_fixed64, put_varint64};
use crate::util::comparator::Comparator;
use std::cmp::Ordering;
use std::fmt::Display;

pub type SequenceNumber = u64;

#[inline]
pub fn extract_user_key(internal_key: &[u8]) -> &[u8] {
    let size = internal_key.len();
    assert!(size >= 8);
    &internal_key[..size - 8]
}

#[derive(Debug, Clone)]
#[repr(u8)]
pub enum ValueType {
    TypeDeletion = 0,
    TypeValue = 1,
}

const TypeForSeek: ValueType = ValueType::TypeValue;

#[derive(Debug)]
pub struct ParseInternalKey<'a> {
    pub user_key: &'a [u8],
    pub seq: SequenceNumber,
    pub value_type: ValueType,
}

fn pack_sequence_and_type(seq: SequenceNumber, t: u8) -> u64 {
    seq << 8 | t as u64
}

impl<'a> ParseInternalKey<'a> {
    pub fn new(user_key: &'a [u8], seq: SequenceNumber, value_type: ValueType) -> Self {
        ParseInternalKey {
            user_key,
            seq,
            value_type,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut result = Vec::new();
        result.extend_from_slice(self.user_key);
        result.append(&mut put_varint64(pack_sequence_and_type(
            self.seq,
            self.value_type.clone() as u8,
        )));
        result
    }
}

#[derive(Debug)]
pub struct InternalKey {
    rep: Vec<u8>,
}

impl Display for InternalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InternalKey({:?})", self.rep)
    }
}

impl InternalKey {
    pub fn new(user_key: &[u8], s: SequenceNumber, t: ValueType) -> InternalKey {
        let parsed_key = ParseInternalKey::new(user_key, s, t);
        let rep = parsed_key.encode();
        InternalKey { rep }
    }

    pub fn encode(&self) -> &[u8] {
        &self.rep
    }
    pub fn decode_from(s: &[u8]) -> InternalKey {
        InternalKey { rep: s.to_vec() }
    }

    pub fn user_key(&self) -> &[u8] {
        extract_user_key(&self.rep)
    }
}

pub struct InternalKeyComparator<T: Comparator> {
    user_comparator: T,
}

impl<T: Comparator> Comparator for InternalKeyComparator<T> {
    fn name(&self) -> &str {
        "leveldb.InternalKeyComparator"
    }

    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
        let r = self
            .user_comparator
            .compare(extract_user_key(a), extract_user_key(b));
        if r == Ordering::Equal {
            let anum = decode_fixed64(&a[a.len() - 8..]);
            let bnum = decode_fixed64(&b[b.len() - 8..]);
            if anum > bnum {
                return Ordering::Less;
            } else if anum < bnum {
                return Ordering::Greater;
            }
        }
        r
    }

    fn find_shortest_separator(&self, start: &[u8], limit: &[u8]) -> Option<Vec<u8>> {
        let user_start = extract_user_key(start);
        let user_limit = extract_user_key(limit);

        let mut tmp = self
            .user_comparator
            .find_shortest_separator(start, user_limit)?;
        if tmp.len() < user_start.len()
            && self.user_comparator.compare(user_start, &tmp) == Ordering::Less
        {
            tmp.append(&mut put_fixed64(pack_sequence_and_type(
                SequenceNumber::MAX,
                TypeForSeek as u8,
            )));
            return Some(tmp);
        }
        None
    }

    fn find_short_successor(&self, key: &[u8]) -> Option<Vec<u8>> {
        let user_key = extract_user_key(key);
        let mut tmp = self.user_comparator.find_short_successor(user_key)?;

        if tmp.len() < user_key.len()
            && self.user_comparator.compare(user_key, &tmp) == Ordering::Less
        {
            tmp.append(&mut put_fixed64(pack_sequence_and_type(
                SequenceNumber::MAX,
                TypeForSeek as u8,
            )));
            return Some(tmp);
        }
        None
    }
}

// We construct a char array of the form:
//    klength  varint32               <-- start_
//    userkey  char[klength]          <-- kstart_
//    tag      uint64
//                                    <-- end_
// The array is a suitable MemTable key.
// The suffix starting with "userkey" can be used as an InternalKey.
#[derive(Debug)]
pub struct LookupKey {
    data: Vec<u8>,
    kstart: usize,
    end: usize,
}

impl LookupKey {
    pub fn new(user_key: &[u8], seq: SequenceNumber) -> Self {
        let usize = user_key.len();
        let need = usize + 13;

        let mut data = Vec::with_capacity(need);

        data.append(&mut encode_varint32((usize + 8) as u32));
        let kstart = data.len();
        data.extend_from_slice(user_key);
        data.append(&mut put_varint64(pack_sequence_and_type(
            seq,
            TypeForSeek as u8,
        )));
        let end = data.len();
        LookupKey { data, kstart, end }
    }

    pub fn memtable_key(&self) -> &[u8] {
        &self.data
    }

    pub fn internal_key(&self) -> &[u8] {
        &self.data[self.kstart..self.end]
    }

    pub fn user_key(&self) -> &[u8] {
        &self.data[self.kstart..self.end - 8]
    }
}
