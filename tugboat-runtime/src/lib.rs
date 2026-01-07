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

use crate::start::QemuVmConfig;
use clap::{Parser, Subcommand};
use nix::errno::Errno;
use serde::Deserialize;
use thiserror::Error;

mod config;
mod create;
mod pre;
mod start;
mod status;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("System call Error: {0}")]
    Syscall(#[from] Errno),
    #[error("Failed to setup network: {0}")]
    NetworkSetupFailed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Parser)]
pub struct Args {
    #[arg(
        long,
        help = "Path to the tugboat-inner config file",
        default_value = "/etc/tugboat/inner/config.toml"
    )]
    config: String,

    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(Debug, Subcommand)]
enum SubCommand {
    Start(start::StartArgs),
    Status(status::StatusArgs),
    Create(create::CreateArgs),
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
        SubCommand::Start(run_args) => start::start(config.qemu, run_args).await,
        SubCommand::Status(status_args) => status::status(config.qemu, status_args).await,
        SubCommand::Create(create_args) => create::create(config.qemu, create_args).await,
    }
}
