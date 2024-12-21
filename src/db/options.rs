#[derive(Debug, Default)]
pub struct ReadOptions {
    pub(crate) verify_checksums: bool,
    fill_cache: bool,
    //    snapshot: Option<Snapshot>,
}
