use crate::db::config::NUM_LEVELS;
use crate::db::dbformat::LookupKey;
use crate::db::error::DbError;
use crate::db::options::ReadOptions;
use crate::db::version_edit::{FileMetaData, VersionEdit};
use crate::util::options::Options;
use std::cell::RefCell;
use std::fmt::Display;
use std::rc::{Rc, Weak};
use std::sync::Arc;

pub struct VersionSet {
    //env:
    dbname: String,
    current: Rc<Version>,
}

pub struct Version {
    vset: Weak<VersionSet>,
    next: RefCell<Option<Rc<Version>>>,
    prev: RefCell<Option<Weak<Version>>>,

    files: [Vec<Arc<FileMetaData>>; NUM_LEVELS as usize],
    file_to_compact: Vec<Option<Arc<FileMetaData>>>,
    file_to_compact_level: i32,

    compaction_score: f64,
    compaction_level: i32,
}

struct GetStats {
    seek_file: Option<Rc<FileMetaData>>,
    seek_file_level: i32,
}

enum SaverState {
    kNotFound,
    kFound,
    kDeleted,
    kCorrupt,
}

type FileSet = std::collections::BTreeSet<FileMetaData>;
struct LevelState {
    deleted_files: std::collections::BTreeSet<u64>,
    added_files: FileSet,
}
// A helper class so we can efficiently apply a whole sequence
// of edits to a particular state without creating intermediate
// Versions that contain full copies of the intermediate state.
struct Builder {
    vset: Rc<VersionSet>,
    base: Rc<Version>,
    levels: [Vec<Arc<FileMetaData>>; NUM_LEVELS as usize],
}

impl Builder {
    pub fn new(vset: Rc<VersionSet>, base: Rc<Version>) -> Self {
        todo!()
    }

    pub fn apply(&mut self, edit: &VersionEdit) {
        todo!()
    }
}

struct Saver<'a> {
    state: SaverState,
    user_key: &'a [u8],
    value: &'a [u8],
}
struct State<'a> {
    vset: Rc<VersionSet>,
    saver: Saver<'a>,
    stats: Rc<GetStats>,
    options: Rc<ReadOptions>,
    ikey: &'a [u8],
    last_file_read: Option<Rc<FileMetaData>>,
    last_file_read_level: i32,
    s: DbError,
    found: bool,
}

impl State<'_> {
    pub fn r#match(&mut self, level: i32, f: Rc<FileMetaData>) -> bool {
        todo!()
    }
}
impl Version {
    pub fn new(vset: Rc<VersionSet>) -> Rc<Self> {
        let vset = Rc::downgrade(&vset);
        let v = Rc::new(Version {
            vset,
            next: RefCell::new(None),
            prev: RefCell::new(None),
            files: Default::default(),
            file_to_compact: Vec::new(),
            file_to_compact_level: 0,
            compaction_score: 0.0,
            compaction_level: 0,
        });
        *v.next.borrow_mut() = Some(v.clone());
        *v.prev.borrow_mut() = Some(Rc::downgrade(&v));
        v
    }

    pub fn get(
        &self,
        options: &ReadOptions,
        k: &LookupKey,
        stats: &mut GetStats,
    ) -> Result<&[u8], DbError> {
        todo!()
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        for i in 0..NUM_LEVELS {
            s.push_str("--- level ");
        }
        f.write_str(&s)
    }
}

struct Compaction {
    level: i32,
    max_output_file_size: u64,
    input_version: Option<Rc<Version>>,
    input: [Vec<Arc<FileMetaData>>; 2_usize],
    grandparents: Vec<Arc<FileMetaData>>,
    grandparent_index: i32,
    seen_key: bool,
    overlapped_bytes: u64,
    level_ptrs: [usize; NUM_LEVELS as usize],
    edit: VersionEdit,
}

impl Compaction {
    pub fn new(options: Options, level: i32) -> Self {
        let max_output_file_size = options.max_file_size as u64;
        Compaction {
            level,
            max_output_file_size,
            input_version: None,
            input: Default::default(),
            grandparents: Vec::new(),
            grandparent_index: 0,
            seen_key: false,
            overlapped_bytes: 0,
            level_ptrs: Default::default(),
            edit: VersionEdit::new(),
        }
    }
}
