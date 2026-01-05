// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::run::QemuVmConfig;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::env::VarError;
use thiserror::Error;

mod run;
mod status;

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
    Status(status::StatusArgs),
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
        SubCommand::Run(run_args) => run::run(config.qemu, run_args).await,
        SubCommand::Status(status_args) => status::status(config.qemu, status_args).await,
    }
}
