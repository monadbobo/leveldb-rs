pub trait FilterPolicy {
    fn name(&self) -> &str;
    fn create_filter(&self, keys: &Vec<&[u8]>) -> Vec<u8>;
    fn key_may_match(&self, key: &[u8], filter: &[u8]) -> bool;
}
