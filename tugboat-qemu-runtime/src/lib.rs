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
use tugboat_vm_runtime_interface::error::ErrorKind;

mod cmd;
mod execute;
mod validate;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON Error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TOML Error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("System call Error: {0}")]
    Syscall(#[from] Errno),
    #[error("Failed to setup network: {0}")]
    NetworkSetupFailed(String),
    #[error("QMP operation failed: {0}")]
    Qmp(String),
    #[error("Action failed: {0}")]
    ActionFailed(String),
    #[error("Validation error: {0}")]
    Validation(String),
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

pub fn run() {
    let args = Args::parse();
    let result = (|| -> Result<()> {
        match &args.subcommand {
            SubCommand::Run(run_args) => run::prepare(run_args)?,
            SubCommand::Create(create_args) => create::prepare(create_args)?,
            _ => {}
        }

        let config: Config = tugboat_runtime_common::config::load_toml_config(&args.config)?;
        let Config { qemu } = config;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        runtime.block_on(async move {
            match (args.subcommand, qemu) {
                (SubCommand::Run(run_args), qemu) => run::run(qemu, run_args).await,
                (SubCommand::Status(status_args), qemu) => status::status(qemu, status_args).await,
                (SubCommand::MigrationStatus(status_args), qemu) => {
                    migration_status::status(qemu, status_args).await
                }
                (SubCommand::Create(create_args), qemu) => create::create(qemu, create_args).await,
                (SubCommand::Hotplug(hotplug_args), qemu) => hotplug::run(qemu, hotplug_args).await,
                (SubCommand::Migrate(migrate_args), qemu) => {
                    migrate::migrate(qemu, migrate_args).await
                }
                (SubCommand::MigrateCancel(cancel_args), qemu) => {
                    migrate_cancel::migrate_cancel(qemu, cancel_args).await
                }
                (SubCommand::Start(start_args), _) => start::start(start_args).await,
                (SubCommand::Stop(stop_args), qemu) => stop::stop(qemu, stop_args).await,
            }
        })?;

        Ok(())
    })();

    if let Err(e) = result {
        error!("Runtime error: {e}");
        if let Err(e) = serde_json::to_writer(
            std::io::stdout(),
            &tugboat_vm_runtime_interface::error::Error::from(e),
        ) {
            error!("Failed to serialize error: {e}");
        }
        std::process::exit(1);
    }
}

impl From<Error> for tugboat_vm_runtime_interface::error::Error {
    fn from(value: Error) -> Self {
        match value {
            Error::Io(e) => Self {
                kind: ErrorKind::Io,
                message: e.to_string(),
                details: None,
            },
            Error::Json(e) => Self {
                kind: ErrorKind::Serialization,
                message: e.to_string(),
                details: Some(serde_json::json!({
                    "format": "json",
                    "line": e.line(),
                    "column": e.column(),
                })),
            },
            Error::Toml(e) => Self {
                kind: ErrorKind::Serialization,
                message: e.to_string(),
                details: Some(serde_json::json!({
                    "format": "toml"
                })),
            },
            Error::Syscall(e) => Self {
                kind: ErrorKind::Syscall,
                message: e.to_string(),
                details: Some(serde_json::json!({
                    "errno": e as i32,
                })),
            },
            Error::NetworkSetupFailed(message) => Self {
                kind: ErrorKind::Network,
                message,
                details: None,
            },
            Error::Qmp(message) => Self {
                kind: ErrorKind::VmOperation,
                message,
                details: None,
            },
            Error::ActionFailed(message) => Self {
                kind: ErrorKind::VmOperation,
                message,
                details: None,
            },
            Error::Validation(message) => Self {
                kind: ErrorKind::Validation,
                message,
                details: None,
            },
        }
    }
}

impl From<tugboat_runtime_common::Error> for Error {
    fn from(value: tugboat_runtime_common::Error) -> Self {
        match value {
            tugboat_runtime_common::Error::Io(e) => Self::Io(e),
            tugboat_runtime_common::Error::Json(e) => Self::Json(e),
            tugboat_runtime_common::Error::Toml(e) => Self::Toml(e),
            tugboat_runtime_common::Error::Syscall(e) => Self::Syscall(e),
            tugboat_runtime_common::Error::Validation(message) => Self::Validation(message),
        }
    }
}
