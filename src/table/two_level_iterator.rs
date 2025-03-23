use crate::db::error::DbError;
use crate::db::iterator::DBIterator;
use crate::db::options::ReadOptions;
use crate::table::iterator_wrapper::IteratorWrapper;
use std::mem::zeroed;

pub trait BlockFunction {
    fn block_iterator(
        &self,
        options: &ReadOptions,
        index_value: &[u8],
    ) -> Result<Box<dyn DBIterator>, DbError>;
}

pub struct TwoLevelIterator<'a, 'b, B: BlockFunction> {
    block_function: &'b B,
    options: ReadOptions,
    status: Result<(), DbError>,
    index_iter: IteratorWrapper<'a>,
    data_iter: IteratorWrapper<'a>,
    data_block_handle: Vec<u8>,
}

impl<'a, 'b, B: BlockFunction> TwoLevelIterator<'a, 'b, B> {
    pub fn new(
        index_iter: Box<dyn DBIterator + 'a>,
        block_function: &'b B,
        options: ReadOptions,
    ) -> Self {
        Self {
            block_function,
            options,
            status: Ok(()),
            index_iter: IteratorWrapper::new_with_iterator(Some(index_iter)),
            data_iter: IteratorWrapper::new_with_iterator(None),
            data_block_handle: Vec::new(),
        }
    }

    fn save_error(&mut self, status: Result<(), DbError>) {
        if self.status.is_err() && status.is_err() {
            self.status = status;
        }
    }

    fn set_data_iterator(&mut self, data_iter: Option<Box<dyn DBIterator>>) {
        if let Some(old_iter) = self.data_iter.iter() {
            self.save_error(old_iter.status());
        }
        self.data_iter.set(data_iter);
    }

    fn init_data_block(&mut self) {
        println!("init_data_block: {}", self.index_iter.valid());
        if !self.index_iter.valid() {
            self.set_data_iterator(None);
            return;
        }

        let handle = self.index_iter.value();
        println!("init_data_block value: {:?}", handle);
        if self.data_iter.iter().is_some() && handle == self.data_block_handle.as_slice() {
            return;
        }

        match self.block_function.block_iterator(&self.options, handle) {
            Ok(iter) => {
                self.data_block_handle = handle.to_vec();
                println!("init_data_block: {:?}", handle);
                self.set_data_iterator(Some(iter));
                println!("init_data_block: {:?}", self.data_iter.valid());
            }
            Err(e) => {
                self.save_error(Err(e));
                self.set_data_iterator(None);
            }
        }
    }

    fn skip_empty_data_blocks_forward(&mut self) {
        while self.data_iter.iter().is_none() || !self.data_iter.valid() {
            if !self.index_iter.valid() {
                self.set_data_iterator(None);
                return;
            }
            println!("skip_empty_data_blocks_forward index next");
            self.index_iter.next();
            println!("skip_empty_data_blocks_forward init_data_block");
            self.init_data_block();
            if self.data_iter.iter().is_some() {
                self.data_iter.seek_to_first();
            }
        }
    }

    fn skip_empty_data_blocks_backward(&mut self) {
        while self.data_iter.iter().is_none() || !self.data_iter.valid() {
            if !self.index_iter.valid() {
                self.set_data_iterator(None);
                return;
            }
            self.index_iter.prev();
            self.init_data_block();
            if self.data_iter.iter().is_some() {
                self.data_iter.seek_to_last();
            }
        }
    }
}

impl<'a, 'b, B: BlockFunction> DBIterator for TwoLevelIterator<'a, 'b, B> {
    fn valid(&self) -> bool {
        self.data_iter.valid()
    }

    fn seek_to_first(&mut self) {
        self.index_iter.seek_to_first();
        println!("block seek_to_first: {:?}", self.index_iter.valid());
        self.init_data_block();
        println!("block seek_to_first2: {:?}", self.valid());
        if self.data_iter.iter().is_some() {
            println!("block data seek_to_first");
            self.data_iter.seek_to_first();
            println!("block data seek_to_first end: {:?}", self.valid());
        }
        println!("block data seek_to_first end3: {:?}", self.valid());
        self.skip_empty_data_blocks_forward();
        println!("block data seek_to_first end4: {:?}", self.valid());
    }

    fn seek_to_last(&mut self) {
        self.index_iter.seek_to_last();
        self.init_data_block();
        if self.data_iter.iter().is_some() {
            self.data_iter.seek_to_last();
        }

        self.skip_empty_data_blocks_backward();
    }

    fn seek(&mut self, target: &[u8]) {
        self.index_iter.seek(target);
        self.init_data_block();
        if let Some(iter) = self.data_iter.iter_mut() {
            iter.seek(target);
        }
        self.skip_empty_data_blocks_forward();
    }

    fn next(&mut self) {
        assert!(self.valid());
        self.data_iter.next();
        self.skip_empty_data_blocks_forward();
    }

    fn prev(&mut self) {
        assert!(self.valid());
        self.data_iter.prev();
        self.skip_empty_data_blocks_backward();
    }

    fn key(&self) -> &[u8] {
        assert!(self.valid());
        self.data_iter.key()
    }

    fn value(&self) -> &[u8] {
        self.data_iter.value()
    }

    fn status(&self) -> Result<(), DbError> {
        /*if let Some(e) = &self.status {
            return Err(e.clone());
        }*/

        self.index_iter.status()?;

        // 检查 data_iter 状态(如果存在)
        self.data_iter.iter().map_or(Ok(()), |iter| iter.status())
    }
}

// 对应的创建函数
pub fn new_two_level_iterator<'b, B: BlockFunction>(
    index_iter: Box<dyn DBIterator + 'b>,
    block_function: &'b B,
    options: ReadOptions,
) -> Box<dyn DBIterator + 'b> {
    Box::new(TwoLevelIterator::new(index_iter, block_function, options))
}
