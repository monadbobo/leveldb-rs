pub struct BlockContent {
    pub(crate) data: Vec<u8>,
    cachable: bool,
    pub(crate) heap_allocated: bool,
}
