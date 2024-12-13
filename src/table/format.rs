pub struct BlockContent<'a> {
    data: &'a [u8],
    cachable: bool,
    heap_allocated: bool,
}
