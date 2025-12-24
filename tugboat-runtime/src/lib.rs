use resources::manifests::core::v1::CpuSpec;
use std::env::VarError;
use thiserror::Error;
pub use vm::*;

mod env;
mod vm;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Environment Error: {0}: {1}")]
    Environment(String, VarError),
    #[error("Environment Parse Error: {0}")]
    EnvironmentParseError(String, String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct RuntimeArgs {
    image: String,
    cpu: CpuSpec,
    memory: u64,
    id: String,
}

pub async fn execute<S>(spawner: S, args: RuntimeArgs) -> Result<()>
where
    S: Spawner,
{
    spawner.spawn(args).await
}
