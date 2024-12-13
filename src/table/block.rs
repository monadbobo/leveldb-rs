use crate::table::format::BlockContent;

pub struct Block {
    data: Vec<u8>,
    size: isize,
    restart_offset: u32,
    owned: bool,
}

impl Block {
    pub fn new(block_content: &BlockContent) -> Block {
        todo!()
    }
}
