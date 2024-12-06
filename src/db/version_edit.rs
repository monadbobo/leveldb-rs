use crate::db::config::NUM_LEVELS;
use crate::db::dbformat::{InternalKey, SequenceNumber};
use crate::db::error::DbError;
use crate::db::error::DbError::Corruption;
use crate::db::version_edit::Tag::{
    CompactPointer, Compactor, DeleteFile, LastSequence, LogNumber, NewFile, NextFileNumber,
    PrevLogNumber,
};
use crate::util::coding::{
    get_length_prefixed_slice, get_varint32, get_varint64, put_length_prefixed_slice, put_varint32,
    put_varint64,
};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt::Display;

type DeletedFileSet = BTreeSet<(i32, u64)>;
#[derive(Debug)]
pub struct FileMetaData {
    pub refs: i32,
    pub allowed_seeks: i32,
    pub number: u64,
    pub file_size: u64,
    pub smallest: InternalKey,
    pub largest: InternalKey,
}

// this was c++ code , rewrite to rust :     bool operator()(FileMetaData* f1, FileMetaData* f2) const {
//       int r = internal_comparator->Compare(f1->smallest, f2->smallest);
//       if (r != 0) {
//         return (r < 0);
//       } else {
//         // Break ties by file number
//         return (f1->number < f2->number);
//       }
//     }

impl Eq for FileMetaData {}

impl PartialEq<Self> for FileMetaData {
    fn eq(&self, other: &Self) -> bool {
        todo!()
    }
}

impl PartialOrd<Self> for FileMetaData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        todo!()
    }
}

// impl Ord for FileMetaData {
//     fn cmp(&self, other: &Self) -> Ordering {
//         let r = self.smallest.cmp(&other.smallest);
//         if r != Ordering::Equal {
//             return r;
//         }
//         self.number.cmp(&other.number)
//     }
// }

#[derive(Debug, Default)]
pub struct VersionEdit {
    comparator: String,
    log_number: u64,
    prev_log_number: u64,
    next_file_number: u64,
    last_sequence: SequenceNumber,

    has_comparator: bool,
    has_log_number: bool,
    has_prev_log_number: bool,
    has_next_file_number: bool,
    has_last_sequence: bool,

    pub compact_pointers: Vec<(i32, InternalKey)>,
    pub deleted_files: DeletedFileSet,
    pub new_files: Vec<(i32, FileMetaData)>,
}

#[repr(u8)]
pub enum Tag {
    Compactor = 1,
    LogNumber = 2,
    NextFileNumber = 3,
    LastSequence = 4,
    CompactPointer = 5,
    DeleteFile = 6,
    NewFile = 7,
    PrevLogNumber = 9,
}

impl TryFrom<u8> for Tag {
    type Error = &'static str;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            1 => Ok(Tag::Compactor),
            2 => Ok(Tag::LogNumber),
            3 => Ok(Tag::NextFileNumber),
            4 => Ok(Tag::LastSequence),
            5 => Ok(Tag::CompactPointer),
            6 => Ok(Tag::DeleteFile),
            7 => Ok(Tag::NewFile),
            8 => Ok(Tag::PrevLogNumber),
            _ => Err("Invalid value"),
        }
    }
}

impl VersionEdit {
    pub fn clear(&mut self) {
        self.comparator.clear();
        self.log_number = 0;
        self.prev_log_number = 0;
        self.last_sequence = 0;
        self.next_file_number = 0;
        self.has_comparator = false;
        self.has_log_number = false;
        self.has_prev_log_number = false;
        self.has_next_file_number = false;
        self.has_last_sequence = false;

        self.compact_pointers.clear();
        self.new_files.clear();
        self.deleted_files.clear();
    }

    pub fn new() -> VersionEdit {
        VersionEdit::default()
    }

    pub fn set_comparator_name(&mut self, name: &str) {
        self.has_comparator = true;
        self.comparator = name.to_string();
    }

    pub fn set_log_number(&mut self, log_number: u64) {
        self.has_log_number = true;
        self.log_number = log_number;
    }

    pub fn set_prev_log_number(&mut self, prev_log_number: u64) {
        self.has_prev_log_number = true;
        self.prev_log_number = prev_log_number;
    }

    pub fn set_next_file(&mut self, next_file_number: u64) {
        self.has_next_file_number = true;
        self.next_file_number = next_file_number;
    }

    pub fn set_last_sequence(&mut self, last_sequence: SequenceNumber) {
        self.has_last_sequence = true;
        self.last_sequence = last_sequence;
    }

    pub fn set_compact_pointer(&mut self, level: i32, key: InternalKey) {
        self.compact_pointers.push((level, key));
    }

    pub fn add_file(
        &mut self,
        level: i32,
        file: u64,
        file_size: u64,
        smallest: InternalKey,
        largest: InternalKey,
    ) {
        let meta = FileMetaData {
            refs: 0,
            allowed_seeks: 0,
            number: file,
            file_size,
            smallest,
            largest,
        };

        self.new_files.push((level, meta));
    }

    pub fn remove_file(&mut self, level: i32, file: u64) {
        self.deleted_files.insert((level, file));
    }

    pub fn encode_to(&self) -> Vec<u8> {
        let mut v = Vec::new();

        if self.has_comparator {
            v.append(&mut put_varint32(Compactor as u32));
            v.append(&mut put_length_prefixed_slice(self.comparator.as_bytes()));
        }

        if self.has_log_number {
            v.append(&mut put_varint32(LogNumber as u32));
            v.append(&mut put_varint64(self.log_number));
        }

        if self.has_prev_log_number {
            v.append(&mut put_varint32(PrevLogNumber as u32));
            v.append(&mut put_varint64(self.prev_log_number));
        }

        if self.has_next_file_number {
            v.append(&mut put_varint32(NextFileNumber as u32));
            v.append(&mut put_varint64(self.next_file_number));
        }

        if self.has_last_sequence {
            v.append(&mut put_varint32(LastSequence as u32));
            v.append(&mut put_varint64(self.last_sequence));
        }

        for cp in self.compact_pointers.iter() {
            v.append(&mut put_varint32(CompactPointer as u32));
            v.append(&mut put_varint32(cp.0 as u32));
            v.append(&mut put_length_prefixed_slice(cp.1.encode()));
        }

        for f in self.deleted_files.iter() {
            v.append(&mut put_varint32(DeleteFile as u32));
            v.append(&mut put_varint32(f.0 as u32));
            v.append(&mut put_varint64(f.1));
        }

        for f in self.new_files.iter() {
            v.append(&mut put_varint32(NewFile as u32));
            let meta = &f.1;
            v.append(&mut put_varint32(f.0 as u32));
            v.append(&mut put_varint64(meta.number));
            v.append(&mut put_varint64(meta.file_size));
            v.append(&mut put_length_prefixed_slice(meta.smallest.encode()));
            v.append(&mut put_length_prefixed_slice(meta.largest.encode()));
        }

        v
    }

    fn get_level(src: &[u8]) -> Option<(usize, u32)> {
        let (bytes_read, level) = get_varint32(src)?;
        if level >= NUM_LEVELS {
            return None;
        }
        Some((bytes_read, level))
    }

    fn get_internal_key(src: &[u8]) -> Option<(usize, InternalKey)> {
        let (bytes_read, key) = get_length_prefixed_slice(src)?;
        let key = InternalKey::decode_from(key);
        Some((bytes_read, key))
    }

    pub fn decode_from(&mut self, src: &[u8]) -> Result<(), DbError> {
        self.clear();

        let mut index = 0;
        while index < src.len() {
            let (bytes_read, tag_u) = get_varint32(&src[index..])
                .ok_or_else(|| Corruption("VersionEdit: invalid varint32".to_string()))?;
            index += bytes_read;

            let tag = Tag::try_from(tag_u as u8)
                .map_err(|_| Corruption(format!("VersionEdit: invalid tag({tag_u})")))?;

            match tag {
                Tag::Compactor => {
                    let (bytes_read, comparator) = get_length_prefixed_slice(&src[index..])
                        .ok_or_else(|| {
                            DbError::Corruption("VersionEdit: comparator name".to_string())
                        })?;
                    self.comparator = String::from_utf8_lossy(comparator).to_string();
                    self.has_comparator = true;
                    index += bytes_read;
                }
                Tag::LogNumber => {
                    let (bytes_read, log_number) = get_varint64(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: log_number".to_string()))?;
                    self.log_number = log_number;
                    self.has_log_number = true;
                    index += bytes_read;
                }
                Tag::NextFileNumber => {
                    let (bytes_read, next_file_number) = get_varint64(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: next_file_number".to_string()))?;
                    self.next_file_number = next_file_number;
                    self.has_next_file_number = true;
                    index += bytes_read;
                }
                Tag::LastSequence => {
                    let (bytes_read, last_sequence) = get_varint64(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: last_sequence".to_string()))?;
                    self.last_sequence = last_sequence;
                    self.has_last_sequence = true;
                    index += bytes_read;
                }
                Tag::CompactPointer => {
                    let (bytes_read, level) = Self::get_level(&src[index..]).ok_or_else(|| {
                        Corruption("VersionEdit: compact_pointer level".to_string())
                    })?;
                    index += bytes_read;

                    let (bytes_read, key) =
                        Self::get_internal_key(&src[index..]).ok_or_else(|| {
                            Corruption("VersionEdit: compact_pointer key".to_string())
                        })?;
                    index += bytes_read;

                    self.compact_pointers.push((level as i32, key));
                }
                Tag::DeleteFile => {
                    let (bytes_read, level) = Self::get_level(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: delete_files level".to_string()))?;
                    index += bytes_read;

                    let (bytes_read, file_number) =
                        get_varint64(&src[index..]).ok_or_else(|| {
                            Corruption("VersionEdit: delete_files file_number".to_string())
                        })?;
                    index += bytes_read;

                    self.deleted_files.insert((level as i32, file_number));
                }
                Tag::NewFile => {
                    let (bytes_read, level) = Self::get_level(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: new_file level".to_string()))?;
                    index += bytes_read;

                    let (bytes_read, number) = get_varint64(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: new_file number".to_string()))?;
                    index += bytes_read;

                    let (bytes_read, file_size) = get_varint64(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: new_file file_size".to_string()))?;
                    index += bytes_read;

                    let (bytes_read, smallest) = Self::get_internal_key(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: new_file smallest".to_string()))?;
                    index += bytes_read;

                    let (bytes_read, largest) = Self::get_internal_key(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: new_file largest".to_string()))?;
                    index += bytes_read;

                    self.new_files.push((
                        level as i32,
                        FileMetaData {
                            refs: 0,
                            allowed_seeks: 0,
                            number,
                            file_size,
                            smallest,
                            largest,
                        },
                    ));
                }
                Tag::PrevLogNumber => {
                    let (bytes_read, prev_log_number) = get_varint64(&src[index..])
                        .ok_or_else(|| Corruption("VersionEdit: prev_log_number".to_string()))?;
                    self.prev_log_number = prev_log_number;
                    self.has_prev_log_number = true;
                    index += bytes_read;
                }
            }
        }

        Ok(())
    }
}

impl Display for VersionEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VersionEdit {{")?;
        if self.has_comparator {
            write!(f, "\n  Comparator: ")?;
            write!(f, "{}", self.comparator)?;
        }

        if self.has_log_number {
            write!(f, "\n  LogNumber: ")?;
            write!(f, "{}", self.log_number)?;
        }

        if self.has_prev_log_number {
            write!(f, "\n  PrevLogNumber: ")?;
            write!(f, "{}", self.prev_log_number)?;
        }

        if self.has_next_file_number {
            write!(f, "\n  NextFileNumber: ")?;
            write!(f, "{}", self.next_file_number)?;
        }

        if self.has_last_sequence {
            write!(f, "\n  LastSequence: ")?;
            write!(f, "{}", self.last_sequence)?;
        }

        for c in self.compact_pointers.iter() {
            write!(f, "\n  CompactPointer: ")?;
            write!(f, "{} ", c.0)?;
            write!(f, "{}", c.1)?;
        }

        for d in self.deleted_files.iter() {
            write!(f, "\n  RemoveFiles: ")?;
            write!(f, "{} ", d.0)?;
            write!(f, "{}", d.1)?;
        }

        for n in self.new_files.iter() {
            write!(f, "\n  AddFile: ")?;
            write!(f, "{} ", n.0)?;
            write!(f, "{} ", n.1.number)?;
            write!(f, "{} ", n.1.file_size)?;
            write!(f, "{} .. ", n.1.smallest)?;
            write!(f, "{}", n.1.largest)?;
        }
        write!(f, "\n}}\n")
    }
}

#[cfg(test)]
mod test {
    use crate::db::dbformat::InternalKey;
    use crate::db::dbformat::ValueType::{TypeDeletion, TypeValue};
    use crate::db::version_edit::VersionEdit;

    fn test_encode_decode(edit: &VersionEdit) {
        let encoded = edit.encode_to();
        let mut parsed = VersionEdit::new();
        let d = parsed.decode_from(&encoded);
        assert!(d.is_ok());
        let encoded2 = parsed.encode_to();
        assert_eq!(encoded, encoded2);
    }

    #[test]
    fn test_version_edit() {
        const Big: u64 = 1 << 50;

        let mut edit = VersionEdit::new();
        for i in 0..4 {
            println!("edit: {}", edit);
            test_encode_decode(&edit);
            edit.add_file(
                3,
                Big + 300 + i,
                Big + 400 + i,
                InternalKey::new(b"foo", Big + 500 + i, TypeValue),
                InternalKey::new(b"zoo", Big + 600 + i, TypeDeletion),
            );
            edit.remove_file(4, Big + 700 + i);
            edit.set_compact_pointer(i as i32, InternalKey::new(b"x", Big + 900 + i, TypeValue));
        }

        edit.set_comparator_name("foo");
        edit.set_log_number(Big + 100);
        edit.set_next_file(Big + 200);
        edit.set_last_sequence(Big + 1000);
        test_encode_decode(&edit);
    }
}
