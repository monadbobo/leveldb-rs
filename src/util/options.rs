#[derive(Debug, Clone)]
pub struct Options {
    pub(crate) max_file_size: isize,
    pub(crate) block_restart_interval: isize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_file_size: 4 * 1024 * 1024,
            block_restart_interval: 16,
        }
    }
}
