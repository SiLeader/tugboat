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

use crate::cmd::start;
use crate::execute::vm::QemuVmConfig;
use clap::{Parser, Subcommand};
use cmd::{create, hotplug, migrate, migrate_cancel, migration_status, run, status, stop};
use nix::errno::Errno;
use serde::Deserialize;
use thiserror::Error;
use tracing::error;

mod cmd;
mod config;
mod execute;
mod pre;
mod utils;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("System call Error: {0}")]
    Syscall(#[from] Errno),
    #[error("Failed to setup network: {0}")]
    NetworkSetupFailed(String),
    #[error("QMP operation failed: {0}")]
    Qmp(String),
    #[error("Action failed: {0}")]
    ActionFailed(String),
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
    Run(run::StartArgs),
    Status(status::StatusArgs),
    MigrationStatus(migration_status::MigrationStatusArgs),
    Create(create::CreateArgs),
    Hotplug(hotplug::HotplugArgs),
    Migrate(migrate::MigrateArgs),
    MigrateCancel(migrate_cancel::MigrateCancelArgs),
    Start(start::StartArgs),
    Stop(stop::StopArgs),
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

    if let Err(e) = match args.subcommand {
        SubCommand::Run(run_args) => run::run(config.qemu, run_args).await,
        SubCommand::Status(status_args) => status::status(config.qemu, status_args).await,
        SubCommand::MigrationStatus(status_args) => {
            migration_status::status(config.qemu, status_args).await
        }
        SubCommand::Create(create_args) => create::create(config.qemu, create_args).await,
        SubCommand::Hotplug(hotplug_args) => hotplug::run(config.qemu, hotplug_args).await,
        SubCommand::Migrate(migrate_args) => migrate::migrate(config.qemu, migrate_args).await,
        SubCommand::MigrateCancel(cancel_args) => {
            migrate_cancel::migrate_cancel(config.qemu, cancel_args).await
        }
        SubCommand::Start(start_args) => start::start(start_args).await,
        SubCommand::Stop(stop_args) => stop::stop(config.qemu, stop_args).await,
    } {
        error!("Runtime error: {e}");
        std::process::exit(1);
    }
}
