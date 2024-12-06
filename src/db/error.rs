use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("corruption: {0}")]
    Corruption(String),
    #[error("not supported: {0}")]
    NotSupported(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),
}
