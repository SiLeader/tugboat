use clap::Parser;
mod vm;

pub(crate) enum Error {
    Io(std::io::Error),
    Copy,
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Parser, Debug, Clone)]
pub struct RuntimeArgs {}

pub async fn execute(args: RuntimeArgs) {}
