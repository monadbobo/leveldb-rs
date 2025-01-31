use crate::db::error::DbError;

pub trait DBIterator {
    fn valid(&self) -> bool;
    fn seek_to_first(&mut self);
    fn seek_to_last(&mut self);
    fn seek(&mut self, target: &[u8]);
    fn next(&mut self);
    fn prev(&mut self);
    fn key(&self) -> &[u8];
    fn value(&self) -> &[u8];
    fn status(&self) -> Result<(), DbError>;
}

pub struct EmptyDBIterator {
    status: Result<(), DbError>,
}

impl EmptyDBIterator {
    pub fn new(status: Result<(), DbError>) -> Self {
        Self { status }
    }
}

impl DBIterator for EmptyDBIterator {
    fn valid(&self) -> bool {
        false
    }

    fn seek_to_first(&mut self) {}

    fn seek_to_last(&mut self) {}

    fn seek(&mut self, _target: &[u8]) {}

    fn next(&mut self) {}

    fn prev(&mut self) {}

    fn key(&self) -> &[u8] {
        &[]
    }

    fn value(&self) -> &[u8] {
        &[]
    }

    fn status(&self) -> Result<(), DbError> {
        todo!()
    }
}

pub fn new_error_iterator(status: Result<(), DbError>) -> Box<dyn DBIterator> {
    Box::new(EmptyDBIterator::new(status))
}
