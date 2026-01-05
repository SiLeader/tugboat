use crate::run::QemuVmConfig;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::env::VarError;
use thiserror::Error;

mod run;

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

#[derive(Debug, Parser)]
pub struct Args {
    #[arg(
        long,
        help = "Path to the tugboat-runtime config file",
        default_value = "/etc/tugboat/runtime/config.toml"
    )]
    config: String,

    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(Debug, Subcommand)]
enum SubCommand {
    Run(run::RunArgs),
}

#[derive(Deserialize)]
struct Config {
    qemu: QemuVmConfig,
}

pub async fn run() {
    let args = Args::parse();
    let config = {
        let file = std::fs::read_to_string(args.config).expect("Failed to read config file");
        let config: Config = toml::from_str(&file).expect("Failed to parse config file as TOML");
        config
    };

    match args.subcommand {
        SubCommand::Run(start_args) => run::run(config.qemu, start_args).await,
    }
}
