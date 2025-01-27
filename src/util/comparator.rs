use std::any::Any;
use std::cmp::Ordering;

pub trait Comparator: Any {
    fn name(&self) -> &str;
    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering;
    fn find_shortest_separator(&self, start: &[u8], limit: &[u8]) -> Option<Vec<u8>>;
    fn find_short_successor(&self, key: &[u8]) -> Option<Vec<u8>>;
}

pub struct BytewiseComparatorImpl;

impl Comparator for BytewiseComparatorImpl {
    fn name(&self) -> &str {
        "leveldb.BytewiseComparator"
    }

    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering {
        a.cmp(b)
    }

    fn find_shortest_separator(&self, start: &[u8], limit: &[u8]) -> Option<Vec<u8>> {
        let min_size = std::cmp::min(start.len(), limit.len());
        let mut diff_index = 0;
        while diff_index < min_size && start[diff_index] == limit[diff_index] {
            diff_index += 1;
        }

        if diff_index >= min_size {
            None
        } else {
            let diff_byte = start[diff_index];
            if diff_byte < 0xff && diff_byte + 1 < limit[diff_index] {
                let mut result = start[..diff_index].to_vec();
                result.push(diff_byte + 1);
                return Some(result);
            }
            Some(start[..diff_index + 1].to_vec())
        }
    }

    fn find_short_successor(&self, key: &[u8]) -> Option<Vec<u8>> {
        for (i, k) in key.iter().enumerate() {
            if *k != 0xff {
                let mut result = key[..i].to_vec();
                result.push(k + 1);
                return Some(result);
            }
        }
        None
    }
}
