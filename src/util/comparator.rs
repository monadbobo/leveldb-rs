use std::cmp::Ordering;

pub trait Comparator {
    fn name(&self) -> &str;
    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering;
    fn find_shortest_separator(&self, start: &[u8], limit: &[u8]) -> Option<Vec<u8>>;
    fn find_short_successor(&self, key: &[u8]) -> Option<Vec<u8>>;
}
