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

#[cfg(test)]
mod testing;

#[derive(Debug, Clone, Deserialize)]
pub struct CloudHypervisorVmConfig {
    pub executable: String,
    pub disk_image_location: String,
    pub boot: CloudHypervisorBootConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CloudHypervisorBootConfig {
    pub kernel: Option<String>,
    pub initramfs: Option<String>,
    pub firmware: Option<String>,
}

impl CloudHypervisorVmConfig {
    pub fn get_api_socket_path(&self, id: &str) -> String {
        format!("{}/{}.ch.sock", self.disk_image_location, id)
    }

    pub fn get_event_path(&self, id: &str) -> String {
        format!("{}/{}.events", self.disk_image_location, id)
    }
}

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
    #[error("Cloud Hypervisor API error: {0}")]
    Api(String),
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
        default_value = "/etc/tugboat/runtime/cloud-hypervisor-config.toml"
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

#[derive(Debug, Deserialize)]
struct Config {
    cloud_hypervisor: CloudHypervisorVmConfig,
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
        let Config { cloud_hypervisor } = config;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        runtime.block_on(async move {
            match (args.subcommand, cloud_hypervisor) {
                (SubCommand::Run(run_args), cloud_hypervisor) => {
                    run::run(cloud_hypervisor, run_args).await
                }
                (SubCommand::Status(status_args), cloud_hypervisor) => {
                    status::status(cloud_hypervisor, status_args).await
                }
                (SubCommand::MigrationStatus(status_args), cloud_hypervisor) => {
                    migration_status::status(cloud_hypervisor, status_args).await
                }
                (SubCommand::Create(create_args), cloud_hypervisor) => {
                    create::create(cloud_hypervisor, create_args).await
                }
                (SubCommand::Hotplug(hotplug_args), cloud_hypervisor) => {
                    hotplug::run(cloud_hypervisor, hotplug_args).await
                }
                (SubCommand::Migrate(migrate_args), cloud_hypervisor) => {
                    migrate::migrate(cloud_hypervisor, migrate_args).await
                }
                (SubCommand::MigrateCancel(cancel_args), cloud_hypervisor) => {
                    migrate_cancel::migrate_cancel(cloud_hypervisor, cancel_args).await
                }
                (SubCommand::Start(start_args), _) => start::start(start_args).await,
                (SubCommand::Stop(stop_args), cloud_hypervisor) => {
                    stop::stop(cloud_hypervisor, stop_args).await
                }
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
            Error::Api(message) => Self {
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

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn cloud_hypervisor_config_deserializes_from_toml() {
        let config = toml::from_str::<Config>(
            r#"
[cloud_hypervisor]
executable = "/usr/bin/cloud-hypervisor"
disk_image_location = "/var/lib/tugboat-agent/images"

[cloud_hypervisor.boot]
kernel = "/var/lib/tugboat-agent/vmlinux"
initramfs = "/var/lib/tugboat-agent/initramfs.img"
"#,
        )
        .unwrap();

        assert_eq!(
            config.cloud_hypervisor.executable,
            "/usr/bin/cloud-hypervisor"
        );
        assert_eq!(
            config.cloud_hypervisor.disk_image_location,
            "/var/lib/tugboat-agent/images"
        );
        assert_eq!(
            config.cloud_hypervisor.boot.kernel.as_deref(),
            Some("/var/lib/tugboat-agent/vmlinux")
        );
        assert_eq!(
            config.cloud_hypervisor.boot.initramfs.as_deref(),
            Some("/var/lib/tugboat-agent/initramfs.img")
        );
        assert_eq!(config.cloud_hypervisor.boot.firmware, None);
    }

    #[test]
    fn cloud_hypervisor_sample_config_deserializes_from_toml() {
        let config = toml::from_str::<Config>(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sample-configs/runtime/cloud-hypervisor-config.toml"
        )))
        .unwrap();

        assert_eq!(
            config.cloud_hypervisor.executable,
            "/usr/bin/cloud-hypervisor"
        );
        assert_eq!(
            config.cloud_hypervisor.disk_image_location,
            "/var/lib/tugboat-agent/images"
        );
        assert_eq!(
            config.cloud_hypervisor.boot.kernel.as_deref(),
            Some("/var/lib/tugboat-agent/vmlinux")
        );
        assert_eq!(config.cloud_hypervisor.boot.initramfs, None);
        assert_eq!(config.cloud_hypervisor.boot.firmware, None);
    }

    #[test]
    fn cloud_hypervisor_uefi_config_deserializes_from_toml() {
        let config = toml::from_str::<Config>(
            r#"
[cloud_hypervisor]
executable = "/usr/bin/cloud-hypervisor"
disk_image_location = "/var/lib/tugboat-agent/images"

[cloud_hypervisor.boot]
firmware = "/usr/share/OVMF/OVMF.fd"
"#,
        )
        .unwrap();

        assert_eq!(
            config.cloud_hypervisor.boot.firmware.as_deref(),
            Some("/usr/share/OVMF/OVMF.fd")
        );
        assert_eq!(config.cloud_hypervisor.boot.kernel, None);
        assert_eq!(config.cloud_hypervisor.boot.initramfs, None);
    }
}
