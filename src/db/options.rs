#[derive(Debug, Default)]
pub struct ReadOptions {
    verify_checksums: bool,
    fill_cache: bool,
    //    snapshot: Option<Snapshot>,
}