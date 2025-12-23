use clap::Parser;
use thiserror::Error;

mod vm;

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Parser, Debug, Clone)]
pub struct RuntimeArgs {}

pub async fn execute(args: RuntimeArgs) {}
