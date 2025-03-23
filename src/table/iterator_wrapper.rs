use crate::db::error::DbError;
use crate::db::iterator::DBIterator;

pub struct IteratorWrapper<'a> {
    iter: Option<Box<dyn DBIterator + 'a>>,
    valid: bool,
    key: &'a [u8],
}

impl<'a> IteratorWrapper<'a> {
    pub fn new() -> Self {
        Self {
            iter: None,
            valid: false,
            key: &[],
        }
    }

    pub fn new_with_iterator(iter: Option<Box<dyn DBIterator + 'a>>) -> Self {
        let mut wrapper = Self::new();
        wrapper.set(iter);
        wrapper
    }

    pub fn set(&mut self, iter: Option<Box<dyn DBIterator + 'a>>) {
        self.iter = iter;
        if self.iter.is_none() {
            self.valid = false;
        } else {
            println!("iterator wrapper set");
            self.update();
        }
    }

    pub fn iter(&self) -> Option<&dyn DBIterator> {
        self.iter.as_deref()
    }

    pub fn iter_mut(&mut self) -> Option<&mut (dyn DBIterator + 'a)> {
        self.iter.as_deref_mut()
    }

    pub fn valid(&self) -> bool {
        self.valid
    }

    pub fn key(&self) -> &[u8] {
        assert!(self.valid());
        &self.key
    }

    pub fn value(&self) -> &[u8] {
        assert!(self.valid());
        self.iter.as_ref().unwrap().value()
    }

    pub fn status(&self) -> Result<(), DbError> {
        self.iter.as_ref().unwrap().status()
    }

    pub fn next(&mut self) {
        assert!(self.iter.is_some());
        self.iter.as_mut().unwrap().next();
        println!("iterator wrapper next");
        self.update();
    }

    pub fn prev(&mut self) {
        self.iter.as_mut().unwrap().prev();
        println!("iterator wrapper prev");
        self.update();
    }

    pub fn seek(&mut self, target: &[u8]) {
        self.iter.as_mut().unwrap().seek(target);
        println!("iterator wrapper seek");
        self.update();
    }

    pub fn seek_to_first(&mut self) {
        assert!(self.iter.is_some());
        self.iter.as_mut().unwrap().seek_to_first();
        println!("iterator wrapper seek_to_first");
        self.update();
    }

    pub fn seek_to_last(&mut self) {
        assert!(self.iter.is_some());
        self.iter.as_mut().unwrap().seek_to_last();
        println!("iterator wrapper seek_to_last");
        self.update();
    }

    fn update(&mut self) {
        assert!(self.iter.is_some());
        if let Some(iter) = self.iter.as_ref() {
            println!("update iter");
            self.valid = iter.valid();
            if self.valid {
                let data: &[u8] = unsafe {
                    //data
                    std::mem::transmute(iter.key())
                };
                self.key = data;
            }
        }
    }
}
