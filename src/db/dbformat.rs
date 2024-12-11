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

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(u8)]
pub enum ValueType {
    TypeDeletion = 0,
    TypeValue = 1,
}

impl From<u8> for ValueType {
    fn from(v: u8) -> Self {
        match v {
            0 => ValueType::TypeDeletion,
            1 => ValueType::TypeValue,
            _ => panic!("Invalid ValueType"),
        }
    }
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

fn append_internal_key(key: &ParseInternalKey) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(key.user_key);
    result.append(&mut put_fixed64(pack_sequence_and_type(
        key.seq,
        key.value_type.clone() as u8,
    )));
    result
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

impl Display for ParseInternalKey<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "'{}' @ {} : {}",
            String::from_utf8_lossy(self.user_key),
            self.seq,
            self.value_type.clone() as u8
        )
    }
}

pub fn parse_internal_key(internal_key: &[u8]) -> Option<ParseInternalKey> {
    let size = internal_key.len();
    if size < 8 {
        return None;
    }
    assert!(size >= 8);
    let num = decode_fixed64(&internal_key[size - 8..]);
    // convert num to enum(u8) ValueType

    let c = num as u8;
    let seq = num >> 8;
    Some(ParseInternalKey {
        user_key: &internal_key[..size - 8],
        seq,
        value_type: c.into(),
    })
}
#[derive(Debug)]
pub struct InternalKey {
    rep: Vec<u8>,
}

impl Display for InternalKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match parse_internal_key(&self.rep) {
            Some(parsed) => write!(f, "{parsed}"),
            None => write!(f, "(bad)"),
        }
    }
}

impl InternalKey {
    pub fn new(user_key: &[u8], s: SequenceNumber, t: ValueType) -> InternalKey {
        InternalKey {
            rep: append_internal_key(&ParseInternalKey::new(user_key, s, t)),
        }
    }

    pub fn encode(&self) -> &[u8] {
        &self.rep
    }
    pub fn decode_from(s: &[u8]) -> Option<InternalKey> {
        if s.is_empty() {
            return None;
        }
        Some(InternalKey { rep: s.to_vec() })
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
            .find_shortest_separator(user_start, user_limit)?;
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

//test
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::dbformat::ValueType::{TypeDeletion, TypeValue};
    use crate::util::comparator::BytewiseComparatorImpl;

    fn i_key(user_key: &[u8], seq: SequenceNumber, vt: ValueType) -> Vec<u8> {
        append_internal_key(&ParseInternalKey::new(user_key, seq, vt))
    }

    fn shorten(a: &[u8], b: &[u8]) -> Vec<u8> {
        let c = InternalKeyComparator {
            user_comparator: BytewiseComparatorImpl,
        };
        match c.find_shortest_separator(a, b) {
            Some(v) => v,
            None => a.to_vec(),
        }
    }

    fn short_successor(a: &[u8]) -> Vec<u8> {
        let c = InternalKeyComparator {
            user_comparator: BytewiseComparatorImpl,
        };
        match c.find_short_successor(a) {
            Some(v) => v,
            None => a.to_vec(),
        }
    }

    fn test_key(key: &[u8], seq: u64, vt: ValueType) {
        let encoded = i_key(key, seq, vt.clone());
        let empty = String::default();
        let pk = parse_internal_key(encoded.as_slice()).unwrap();
        assert_eq!(key, pk.user_key);
        assert_eq!(seq, pk.seq);
        assert_eq!(vt, pk.value_type);
    }

    #[test]
    fn test_internal_key_encode_decode() {
        let keys = ["", "k", "hello", "longggggggggggggggggggggg"];
        let seq = [
            1,
            2,
            3,
            (1_u64 << 8) - 1,
            1_u64 << 8,
            (1_u64 << 8) + 1,
            (1_u64 << 16) - 1,
            1_u64 << 16,
            (1_u64 << 16) + 1,
            (1_u64 << 32) - 1,
            1_u64 << 32,
            (1_u64 << 32) + 1,
        ];
        for k in keys {
            for s in seq {
                test_key(k.as_bytes(), s, TypeValue);
                test_key("hello".as_bytes(), 1, TypeDeletion);
            }
        }
    }

    #[test]
    fn test_internal_key_decodeFrom_empty() {
        let k = InternalKey::decode_from(&[]);
        assert!(k.is_none());
    }

    #[test]
    fn test_internal_key_short_separator() {
        // When user keys are same
        assert_eq!(
            i_key("foo".as_bytes(), 100, TypeValue),
            shorten(
                &i_key("foo".as_bytes(), 100, TypeValue),
                &i_key("foo".as_bytes(), 99, TypeValue)
            )
        );
        assert_eq!(
            i_key("foo".as_bytes(), 100, TypeValue),
            shorten(
                &i_key("foo".as_bytes(), 100, TypeValue),
                &i_key("foo".as_bytes(), 101, TypeValue)
            )
        );
        assert_eq!(
            i_key("foo".as_bytes(), 100, TypeValue),
            shorten(
                &i_key("foo".as_bytes(), 100, TypeValue),
                &i_key("foo".as_bytes(), 100, TypeValue)
            )
        );
        assert_eq!(
            i_key("foo".as_bytes(), 100, TypeValue),
            shorten(
                &i_key("foo".as_bytes(), 100, TypeValue),
                &i_key("foo".as_bytes(), 100, TypeDeletion)
            )
        );

        // When user keys are misordered
        assert_eq!(
            i_key("foo".as_bytes(), 100, TypeValue),
            shorten(
                &i_key("foo".as_bytes(), 100, TypeValue),
                &i_key("bar".as_bytes(), 99, TypeValue)
            )
        );

        // When user keys are different, but correctly ordered
        assert_eq!(
            i_key("g".as_bytes(), SequenceNumber::MAX, TypeForSeek),
            shorten(
                &i_key("foo".as_bytes(), 100, TypeValue),
                &i_key("hello".as_bytes(), 200, TypeValue)
            )
        );

        // When start user key is prefix of limit user key
        assert_eq!(
            i_key("foo".as_bytes(), 100, TypeValue),
            shorten(
                &i_key("foo".as_bytes(), 100, TypeValue),
                &i_key("foobar".as_bytes(), 200, TypeValue)
            )
        );

        // When limit user key is prefix of start user key
        assert_eq!(
            i_key("foobar".as_bytes(), 100, TypeValue),
            shorten(
                &i_key("foobar".as_bytes(), 100, TypeValue),
                &i_key("foo".as_bytes(), 200, TypeValue)
            )
        );
    }

    #[test]
    fn test_internal_key_shortest_successor() {
        assert_eq!(
            i_key("g".as_bytes(), SequenceNumber::MAX, TypeForSeek),
            short_successor(&i_key("foo".as_bytes(), 100, TypeValue))
        );
        assert_eq!(
            i_key(b"\xff\xff", 100, TypeValue),
            short_successor(&i_key(b"\xff\xff", 100, TypeValue))
        );
    }

    #[test]
    fn test_parsed_internal_key_debug_string() {
        let key = ParseInternalKey::new("The \"key\" in 'single quotes'".as_bytes(), 42, TypeValue);
        assert_eq!(
            format!("{key}"),
            "'The \"key\" in 'single quotes'' @ 42 : 1"
        );
    }

    #[test]
    fn test_internal_key_debug_string() {
        let key = InternalKey::new("The \"key\" in 'single quotes'".as_bytes(), 42, TypeValue);
        assert_eq!(
            format!("{key}"),
            "'The \"key\" in 'single quotes'' @ 42 : 1"
        );
    }
}
