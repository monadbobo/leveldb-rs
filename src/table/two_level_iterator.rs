use crate::db::error::DbError;
use crate::db::iterator::DBIterator;
use crate::db::options::ReadOptions;

pub trait BlockFunction {
    fn new_iterator(
        &self,
        options: &ReadOptions,
        index_value: &[u8],
    ) -> Result<Box<dyn DBIterator>, DbError>;
}

pub struct TwoLevelIterator {}
